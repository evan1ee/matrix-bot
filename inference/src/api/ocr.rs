use axum::{body::Bytes, extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::AppState;

/// The OCR wire response. `text` is the blocks joined by newlines, which is
/// what every caller actually consumes; `blocks` carries the per-block scores
/// for callers that want them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrScanBody {
    pub text: String,
    pub blocks: Vec<OcrScanBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrScanBlock {
    pub text: String,
    pub score: f32,
}

/// Maps an `OcrError` to the HTTP status that describes whose fault it is.
///
/// `ImageDecode` is a client error -- the caller sent bytes that are not a
/// decodable image -- so it is `400`, not `500`. `MissingModel` and
/// `ModelDownload` mean the server isn't ready to serve OCR at all, so both
/// are `503`. `Inference` and `Join` are genuine server-side failures.
///
/// Matched exhaustively on purpose: a new `OcrError` variant must be given a
/// status here explicitly, rather than silently falling back to `500`.
fn status_for(err: &crate::ocr::OcrError) -> StatusCode {
    use crate::ocr::OcrError;

    match err {
        OcrError::ImageDecode(_) => StatusCode::BAD_REQUEST,
        OcrError::MissingModel(_) => StatusCode::SERVICE_UNAVAILABLE,
        OcrError::ModelDownload(_) => StatusCode::SERVICE_UNAVAILABLE,
        OcrError::Inference(_) => StatusCode::INTERNAL_SERVER_ERROR,
        OcrError::Join(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `POST /ocr` — raw image bytes in, recognised text out.
///
/// The body is the image exactly as the caller received it; the caller does no
/// decoding, which is what keeps the `image` crate out of the bot.
pub async fn scan(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<OcrScanBody>, (StatusCode, String)> {
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Request body is empty; expected raw image bytes".to_string(),
        ));
    }

    if !crate::ocr::models_present(&state.config.ocr_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "OCR models are not installed on this inference server".to_string(),
        ));
    }

    let result = crate::ocr::run_ocr(body.to_vec(), state.config.ocr_model_dir.clone())
        .await
        .map_err(|e| {
            warn!("PaddleOCR inference failed: {:?}", e);
            (status_for(&e), e.to_string())
        })?;

    Ok(Json(OcrScanBody {
        text: result.combined_text,
        blocks: result
            .blocks
            .into_iter()
            .map(|b| OcrScanBlock {
                text: b.text,
                score: b.score,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use std::sync::Arc;

    const OCR_SCAN_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/ocr_scan.json"
    ));

    #[test]
    fn test_ocr_scan_body_matches_the_shared_wire_contract() {
        let body = OcrScanBody {
            text: "SHAKE SHACK\nTotal $16.50".to_string(),
            blocks: vec![
                OcrScanBlock {
                    text: "SHAKE SHACK".to_string(),
                    score: 0.5,
                },
                OcrScanBlock {
                    text: "Total $16.50".to_string(),
                    score: 0.75,
                },
            ],
        };

        let expected: serde_json::Value =
            serde_json::from_str(OCR_SCAN_CONTRACT).expect("the contract is valid JSON");
        assert_eq!(serde_json::to_value(&body).unwrap(), expected);
    }

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
                voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
                tts_model_dir: std::path::PathBuf::from("/nonexistent/tts/models"),
            }),
        }
    }

    #[tokio::test]
    async fn test_empty_body_is_rejected_as_bad_request() {
        let result = scan(State(state_with_missing_models()), Bytes::new()).await;

        let err = result.expect_err("an empty body should not be accepted");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_missing_models_report_service_unavailable_not_a_crash() {
        let result = scan(
            State(state_with_missing_models()),
            Bytes::from_static(b"notanimage"),
        )
        .await;

        let err = result.expect_err("absent models should be an error");
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_status_for_image_decode_is_bad_request() {
        let err = crate::ocr::OcrError::ImageDecode("bad bytes".to_string());
        assert_eq!(status_for(&err), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_status_for_missing_model_is_service_unavailable() {
        let err = crate::ocr::OcrError::MissingModel("det model".to_string());
        assert_eq!(status_for(&err), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_status_for_model_download_is_service_unavailable() {
        let err = crate::ocr::OcrError::ModelDownload("network error".to_string());
        assert_eq!(status_for(&err), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_status_for_inference_is_internal_server_error() {
        let err = crate::ocr::OcrError::Inference("onnx runtime error".to_string());
        assert_eq!(status_for(&err), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_status_for_join_is_internal_server_error() {
        let err = crate::ocr::OcrError::Join("worker panicked".to_string());
        assert_eq!(status_for(&err), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn test_post_ocr_through_the_router_with_empty_body_returns_400() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let router = crate::api::build_router(state_with_missing_models());

        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/ocr")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
