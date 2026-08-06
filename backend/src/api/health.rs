use axum::{extract::State, Json};
use serde::Serialize;

use crate::api::auth::ApiState;

use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
    pub db_status: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "System health status", body = HealthResponse)
    )
)]
pub async fn health_check(State(state): State<ApiState>) -> Json<HealthResponse> {
    let db_ok = sqlx::query("SELECT 1").execute(&state.db.pool).await.is_ok();

    Json(HealthResponse {
        status: "ok".to_string(),
        service: "matrix-bot-api".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        db_status: if db_ok { "connected".to_string() } else { "disconnected".to_string() },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_health_check() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let res = health_check(State(state)).await;
        assert_eq!(res.status, "ok");
        assert_eq!(res.db_status, "connected");
    }
}
