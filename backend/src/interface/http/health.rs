//! Liveness and readiness probes.
//!
//! Deliberately two different questions. `/health/live` answers "is the
//! process alive at all" and touches nothing else — a container orchestrator
//! restarts the process when this fails, which cannot fix a downstream outage
//! (a dead database, a homeserver that stopped answering sync) and would only
//! thrash the process while making it worse. `/health/ready` answers "should
//! traffic be sent here" and checks the two things that make this bot actually
//! useful: the database and a fresh Matrix sync.
//!
//! Neither route requires auth and neither lives under `/api/v1` — they are
//! infrastructure surface, not the application API, and a load balancer or
//! orchestrator probing them should not need a session.

use axum::{extract::State, http::StatusCode, Json};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::interface::http::auth::ApiState;

/// How stale the last completed Matrix sync may be before readiness fails.
const MATRIX_SYNC_MAX_AGE: Duration = Duration::seconds(120);

#[derive(Serialize, ToSchema)]
pub struct LiveResponse {
    pub status: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct ReadyChecks {
    pub database: DatabaseCheck,
    pub matrix_sync: MatrixSyncCheck,
}

#[derive(Serialize, ToSchema)]
pub struct DatabaseCheck {
    pub ok: bool,
}

#[derive(Serialize, ToSchema)]
pub struct MatrixSyncCheck {
    pub ok: bool,
    #[schema(value_type = Option<String>)]
    pub last_sync_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, ToSchema)]
pub struct ReadyResponse {
    pub status: &'static str,
    pub checks: ReadyChecks,
}

/// Always `200`; touches no dependency. Used by the container runtime to
/// decide whether to restart the process.
#[utoipa::path(
    get,
    path = "/health/live",
    responses(
        (status = 200, description = "The process is alive", body = LiveResponse),
    )
)]
pub async fn live() -> Json<LiveResponse> {
    Json(LiveResponse { status: "live" })
}

/// `200` when the database answers and the Matrix sync loop completed a sync
/// within [`MATRIX_SYNC_MAX_AGE`]; `503` otherwise. Used to decide whether to
/// route traffic here, never to restart the process.
#[utoipa::path(
    get,
    path = "/health/ready",
    responses(
        (status = 200, description = "Ready to serve traffic", body = ReadyResponse),
        (status = 503, description = "Not ready", body = ReadyResponse),
    )
)]
pub async fn ready(State(state): State<ApiState>) -> (StatusCode, Json<ReadyResponse>) {
    let db_ok = sqlx::query_scalar!(r#"SELECT 1 AS "ok!""#)
        .fetch_one(&state.db.pool)
        .await
        .is_ok();

    let last_sync_at = state.sync_health.last_sync();
    let matrix_ok = state.sync_health.is_fresh(Utc::now(), MATRIX_SYNC_MAX_AGE);

    let ready = db_ok && matrix_ok;

    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(ReadyResponse {
            status: if ready { "ready" } else { "not_ready" },
            checks: ReadyChecks {
                database: DatabaseCheck { ok: db_ok },
                matrix_sync: MatrixSyncCheck {
                    ok: matrix_ok,
                    last_sync_at,
                },
            },
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    async fn state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    fn router(state: ApiState) -> Router {
        Router::new()
            .route("/health/live", get(live))
            .route("/health/ready", get(ready))
            .with_state(state)
    }

    async fn get_status_and_body(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, body)
    }

    #[tokio::test]
    async fn test_live_is_always_200() {
        let app = router(state().await);

        let (status, body) = get_status_and_body(app, "/health/live").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "live");
    }

    #[tokio::test]
    async fn test_ready_is_503_when_never_synced() {
        let app = router(state().await);

        let (status, body) = get_status_and_body(app, "/health/ready").await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "not_ready");
        assert_eq!(body["checks"]["matrix_sync"]["ok"], false);
        assert_eq!(body["checks"]["database"]["ok"], true);
    }

    #[tokio::test]
    async fn test_ready_is_200_after_a_recorded_sync() {
        let state = state().await;
        state.sync_health.record_sync(Utc::now());
        let app = router(state);

        let (status, body) = get_status_and_body(app, "/health/ready").await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ready");
        assert_eq!(body["checks"]["matrix_sync"]["ok"], true);
        assert!(body["checks"]["matrix_sync"]["last_sync_at"].is_string());
    }

    #[tokio::test]
    async fn test_ready_is_503_when_database_is_closed() {
        let state = state().await;
        state.sync_health.record_sync(Utc::now());
        state.db.pool.close().await;
        let app = router(state);

        let (status, body) = get_status_and_body(app, "/health/ready").await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["status"], "not_ready");
        assert_eq!(body["checks"]["database"]["ok"], false);
    }
}
