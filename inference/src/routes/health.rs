use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use super::AppState;

/// Per-capability model presence. A capability reports `false` when its model
/// files are absent, which is how an operator tells "not installed" from
/// "installed but failing".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResponse {
    pub stt: bool,
    pub tts: bool,
    pub ocr: bool,
}

/// Reports which models are installed. Each flag is that capability's own
/// presence check (`stt`/`ocr`: `models_present`; `tts`: `model_present`).
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        stt: crate::sherpa::models_present(&state.config.voice_model_dir),
        tts: crate::kokoro::model_present(&state.config.tts_model_dir),
        ocr: crate::paddle::models_present(&state.config.ocr_model_dir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_health_reports_ocr_absent_when_models_are_missing() {
        let config = InferenceConfig {
            port: 0,
            ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
            voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
            tts_model_dir: std::path::PathBuf::from("/nonexistent/tts/models"),
        };
        let state = AppState {
            config: Arc::new(config),
        };

        let Json(body) = health(State(state)).await;

        assert_eq!(
            body,
            HealthResponse {
                stt: false,
                tts: false,
                ocr: false,
            }
        );
    }

    #[tokio::test]
    async fn test_get_health_through_the_router_returns_200_and_the_body() {
        use axum::body::{to_bytes, Body};
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let config = InferenceConfig {
            port: 0,
            ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
            voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
            tts_model_dir: std::path::PathBuf::from("/nonexistent/tts/models"),
        };
        let state = AppState {
            config: Arc::new(config),
        };
        let router = crate::routes::build_router(state);

        let response = router
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body: HealthResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            body,
            HealthResponse {
                stt: false,
                tts: false,
                ocr: false,
            }
        );
    }
}
