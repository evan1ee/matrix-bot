use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use futures_util::stream;
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;
use tracing::info;
use uuid::Uuid;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::modules::mcp::service::McpService;

#[derive(Debug, Deserialize)]
pub struct McpQuery {
    pub session_id: Option<String>,
}

pub fn mcp_routes() -> Router<ApiState> {
    Router::new()
        .route("/sse", get(mcp_sse_handler))
        .route("/messages", post(mcp_message_handler))
        .route("/", post(mcp_direct_handler))
}

/// GET /api/v1/mcp/sse — Establishes the MCP SSE transport connection.
pub async fn mcp_sse_handler(
    State(_state): State<ApiState>,
    user: AuthUser,
) -> Result<Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let session_id = Uuid::new_v4().to_string();
    info!(
        "MCP SSE connection established for owner '{}' (session: {})",
        user.owner_id(),
        session_id
    );

    let endpoint_path = format!("/api/v1/mcp/messages?session_id={}", session_id);
    let initial_event = Event::default().event("endpoint").data(endpoint_path);

    // Initial endpoint announcement followed by periodic keep-alives
    let stream = stream::once(async move { Ok(initial_event) });

    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// POST /api/v1/mcp/messages — Receives JSON-RPC messages from MCP clients in SSE session mode.
pub async fn mcp_message_handler(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(_query): Query<McpQuery>,
    Json(req): Json<JsonRpcRequest>,
) -> Result<Json<Option<JsonRpcResponse>>, ApiError> {
    let owner_id = user.owner_id();
    let service = McpService::new(&state.db.pool, Some(&state.broadcaster), &owner_id);
    let resp = service.handle_request(req).await;
    Ok(Json(resp))
}

/// POST /api/v1/mcp — Direct single-endpoint JSON-RPC handler (no SSE required).
pub async fn mcp_direct_handler(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(req): Json<JsonRpcRequest>,
) -> Result<Json<Option<JsonRpcResponse>>, ApiError> {
    let owner_id = user.owner_id();
    let service = McpService::new(&state.db.pool, Some(&state.broadcaster), &owner_id);
    let resp = service.handle_request(req).await;
    Ok(Json(resp))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_direct_handler_ping() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let service = McpService::new(&pool, None, "test_owner");

        let req = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: Some(json!(42)),
            method: "ping".to_string(),
            params: None,
        };

        let resp = service.handle_request(req).await.unwrap();
        assert_eq!(resp.id, json!(42));
        assert!(resp.error.is_none());
    }
}
