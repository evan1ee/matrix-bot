use axum::{
    extract::{Multipart, State},
    Json,
};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use utoipa::ToSchema;

use crate::interface::http::auth::types::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OcrScanResponse {
    pub raw_text: String,
    pub blocks: Vec<OcrBlockDto>,
    pub structured: Option<ReceiptExtractionDto>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct OcrBlockDto {
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReceiptExtractionDto {
    pub merchant: Option<String>,
    pub amount: Option<String>,
    pub currency: Option<String>,
    pub date: Option<String>,
    pub category_suggestion: Option<String>,
}

/// Scans an uploaded receipt image using local PaddleOCR, extracting text and receipt fields.
#[utoipa::path(
    post,
    path = "/api/v1/ocr/scan",
    tag = "OCR",
    responses(
        (status = 200, description = "Image OCR scan succeeded", body = OcrScanResponse),
        (status = 400, description = "Invalid or empty image uploaded"),
        (status = 503, description = "OCR engine is disabled or models missing")
    ),
    security(("session_cookie" = []))
)]
pub async fn scan_image(
    _user: AuthUser,
    State(state): State<ApiState>,
    mut multipart: Multipart,
) -> Result<Json<OcrScanResponse>, ApiError> {
    let snapshot = state.services.app_config.snapshot();
    let ocr_enabled =
        if snapshot.is_customized(crate::domain::config::SettingKey::InferenceOcrEnabled) {
            snapshot.inference_ocr_enabled()
        } else {
            state.ocr_config.enabled
        };
    if !ocr_enabled {
        return Err(ApiError::service_unavailable(
            "OCR service is disabled in server configuration",
        ));
    }

    let mut image_bytes: Option<Vec<u8>> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read multipart payload: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "image" || name.is_empty() {
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to read image bytes: {}", e)))?;
            image_bytes = Some(bytes.to_vec());
            break;
        }
    }

    let bytes = image_bytes
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::bad_request("No image file provided in multipart request"))?;

    info!("Running web OCR scan on {} image bytes", bytes.len());

    let effective_ocr = state
        .ocr
        .with_base_url_override(Some(snapshot.inference_ocr_base_url()).filter(|s| !s.is_empty()));

    let ocr_result = match effective_ocr.scan(bytes).await {
        Ok(scan) => scan,
        Err(err) => return Err(ocr_error_response(err)),
    };

    let blocks: Vec<OcrBlockDto> = ocr_result
        .blocks
        .into_iter()
        .map(|b| OcrBlockDto {
            text: b.text,
            score: b.score,
        })
        .collect();

    let structured = extract_receipt_fields(&ocr_result.text);

    Ok(Json(OcrScanResponse {
        raw_text: ocr_result.text,
        blocks,
        structured: Some(structured),
    }))
}

/// Maps a failed OCR service call onto the status this endpoint's
/// `#[utoipa::path]` annotation promises, without relaying the upstream
/// response body to the caller.
///
/// The inference server's `status_for` (see the inference crate) already
/// distinguishes 400 (undecodable image) from 503 (models not installed)
/// from 500 (inference failure); this function is what keeps that
/// distinction alive across the HTTP hop instead of collapsing every
/// failure into a 500. The full upstream detail — which can contain the
/// `image` crate's internal decode text — goes to the log via `warn!`,
/// never into the response.
fn ocr_error_response(err: crate::infrastructure::inference::ocr::OcrServiceError) -> ApiError {
    use crate::infrastructure::inference::ocr::OcrServiceError;

    match err {
        OcrServiceError::NotConfigured => {
            ApiError::service_unavailable("OCR service is not configured on this server")
        }
        OcrServiceError::Upstream { status, message } => {
            warn!("OCR extraction failed upstream ({}): {}", status, message);
            match status {
                400 => ApiError::bad_request("The uploaded image could not be decoded"),
                503 => ApiError::service_unavailable("The OCR service has no models installed"),
                _ => ApiError::internal("OCR extraction failed"),
            }
        }
        OcrServiceError::Unreachable(detail) => {
            warn!("OCR service unreachable: {}", detail);
            ApiError::internal("OCR extraction failed")
        }
    }
}

impl From<crate::domain::finance::receipt::ExtractedReceipt> for ReceiptExtractionDto {
    fn from(r: crate::domain::finance::receipt::ExtractedReceipt) -> Self {
        Self {
            merchant: r.merchant,
            amount: r.amount,
            currency: r.currency,
            date: r.date,
            category_suggestion: r.category_suggestion,
        }
    }
}

/// Heuristic offline parser extracting key financial fields from raw receipt OCR text.
pub fn extract_receipt_fields(raw_text: &str) -> ReceiptExtractionDto {
    crate::domain::finance::receipt::extract_receipt_fields(raw_text).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::ocr::OcrServiceError;
    use axum::http::StatusCode;

    /// The inference server's 400 (undecodable image) must reach the caller
    /// as 400, not the generic 500 that swallowing every upstream error would
    /// produce, and the response must not carry the raw `image` crate decode
    /// text -- only a fixed, caller-appropriate message belongs here.
    #[test]
    fn test_upstream_400_maps_to_bad_request_without_relaying_the_body() {
        let err = OcrServiceError::Upstream {
            status: 400,
            message: "image crate: Format error decoding Png: invalid checksum".to_string(),
        };

        let api_err = ocr_error_response(err);

        assert_eq!(api_err.status, StatusCode::BAD_REQUEST);
        assert!(!api_err.message.contains("Png"));
        assert!(!api_err.message.contains("checksum"));
    }

    /// The `#[utoipa::path]` annotation on `scan_image` advertises 503 for
    /// "OCR engine is disabled or models missing" -- this is what makes that
    /// annotation true instead of a promise the handler can never keep.
    #[test]
    fn test_upstream_503_maps_to_service_unavailable() {
        let err = OcrServiceError::Upstream {
            status: 503,
            message: "OCR models are not installed on this inference server".to_string(),
        };

        let api_err = ocr_error_response(err);

        assert_eq!(api_err.status, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// Any other upstream status (the inference server's own 500) is a
    /// genuine server fault and stays a 500, with a fixed message rather than
    /// the upstream body.
    #[test]
    fn test_upstream_500_maps_to_internal_without_relaying_the_body() {
        let err = OcrServiceError::Upstream {
            status: 500,
            message: "panicked at inference/src/paddle.rs:42".to_string(),
        };

        let api_err = ocr_error_response(err);

        assert_eq!(api_err.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(!api_err.message.contains("paddle.rs"));
    }

    /// An unreachable service is a server fault, not a client mistake.
    #[test]
    fn test_unreachable_maps_to_internal() {
        let err = OcrServiceError::Unreachable("connection refused".to_string());

        let api_err = ocr_error_response(err);

        assert_eq!(api_err.status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    /// An absent base URL is a deployment state, not an upstream failure, and
    /// keeps its own distinct status from the other two branches.
    #[test]
    fn test_not_configured_maps_to_service_unavailable() {
        let api_err = ocr_error_response(OcrServiceError::NotConfigured);

        assert_eq!(api_err.status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
