use axum::{extract::State, Json};
use serde::Serialize;

use crate::api::auth::ApiState;

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    pub db_status: String,
}

pub async fn health_check(State(state): State<ApiState>) -> Json<HealthResponse> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db.pool).await.is_ok();

    Json(HealthResponse {
        status: "ok".to_string(),
        service: "matrix-bot-api".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        db_status: if db_ok { "connected".to_string() } else { "disconnected".to_string() },
    })
}
