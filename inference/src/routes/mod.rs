use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::config::InferenceConfig;

pub mod health;
pub mod ocr;
pub mod speech;
pub mod transcriptions;

/// Images and audio clips are the payloads here; 32 MB comfortably exceeds a
/// Matrix attachment while still refusing an unbounded upload.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<InferenceConfig>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/ocr", post(ocr::scan))
        .route("/v1/audio/transcriptions", post(transcriptions::transcribe))
        .route("/v1/audio/speech", post(speech::speak))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}
