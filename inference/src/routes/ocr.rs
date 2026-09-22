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

    if !crate::paddle::models_present(&state.config.ocr_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "OCR models are not installed on this inference server".to_string(),
        ));
    }

    let result = crate::paddle::run_ocr(body.to_vec(), state.config.ocr_model_dir.clone())
        .await
        .map_err(|e| {
            warn!("PaddleOCR inference failed: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
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

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
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

    #[tokio::test]
    async fn test_post_ocr_through_the_router_with_empty_body_returns_400() {
        use axum::body::Body;
        use axum::http::Request;
        use tower::ServiceExt;

        let router = crate::routes::build_router(state_with_missing_models());

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
