use axum::{
    extract::DefaultBodyLimit,
    routing::{get, post},
    Router,
};
use std::sync::Arc;

use crate::config::InferenceConfig;

pub mod health;
pub mod ocr;

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
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}
