use axum::{
    extract::{DefaultBodyLimit, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config::InferenceConfig;

pub mod embeddings;
pub mod ocr;
pub mod stt;
pub mod tts;

/// Images and audio clips are the payloads here; 32 MB comfortably exceeds a
/// Matrix attachment while still refusing an unbounded upload.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<InferenceConfig>,
}

/// Per-capability model presence. A capability reports `false` when its model
/// files are absent, which is how an operator tells "not installed" from
/// "installed but failing".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResponse {
    pub stt: bool,
    pub tts: bool,
    pub ocr: bool,
    pub embeddings: bool,
}

/// Reports which models are installed. Each flag is that capability's own
/// presence check (`stt`/`ocr`/`embeddings`: `models_present`; `tts`: `model_present`).
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        stt: crate::stt::models_present(&state.config.voice_model_dir),
        tts: crate::tts::model_present(&state.config.tts_model_dir),
        ocr: crate::ocr::models_present(&state.config.ocr_model_dir),
        embeddings: crate::embeddings::models_present(&state.config.embedding_model_dir),
    })
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ocr", post(ocr::scan))
        .route("/v1/audio/transcriptions", post(stt::transcribe))
        .route("/v1/audio/speech", post(tts::speak))
        .route("/v1/embeddings", post(embeddings::embed))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
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
            embedding_model_dir: std::path::PathBuf::from("/nonexistent/embed/models"),
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
                embeddings: false,
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
            embedding_model_dir: std::path::PathBuf::from("/nonexistent/embed/models"),
        };
        let state = AppState {
            config: Arc::new(config),
        };
        let router = build_router(state);

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
                embeddings: false,
            }
        );
    }
}
