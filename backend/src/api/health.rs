use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::auth::ApiState;

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
        (status = 200, description = "System healthy", body = HealthResponse),
        (status = 503, description = "System degraded or database unavailable", body = HealthResponse)
    )
)]
pub async fn health_check(State(state): State<ApiState>) -> (StatusCode, Json<HealthResponse>) {
    let db_ok = sqlx::query_scalar!(r#"SELECT 1 AS "ok!""#)
        .fetch_one(&state.db.pool)
        .await
        .is_ok();

    let status_code = if db_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(HealthResponse {
            status: if db_ok {
                "ok".to_string()
            } else {
                "degraded".to_string()
            },
            service: "matrix-bot-api".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            db_status: if db_ok {
                "connected".to_string()
            } else {
                "disconnected".to_string()
            },
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    #[tokio::test]
    async fn test_health_check() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let (status, Json(res)) = health_check(State(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(res.status, "ok");
        assert_eq!(res.db_status, "connected");
    }

    #[tokio::test]
    async fn test_health_check_disconnected() {
        let pool = crate::db::pool::test_pool().await;
        pool.close().await;
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let (status, Json(res)) = health_check(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(res.status, "degraded");
        assert_eq!(res.db_status, "disconnected");
    }
}
