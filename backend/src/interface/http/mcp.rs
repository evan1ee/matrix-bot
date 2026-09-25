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

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::modules::mcp::protocol::{JsonRpcRequest, JsonRpcResponse};
use crate::modules::mcp::service::McpService;

use crate::modules::mcp::models::{
    self, CreateMcpTokenRequest, CreateMcpTokenResponse, McpTokenSummary,
};
use crate::modules::mcp::repository::McpTokenRepository;

#[derive(Debug, Deserialize)]
pub struct McpQuery {
    pub session_id: Option<String>,
}

pub fn mcp_routes() -> Router<ApiState> {
    Router::new()
        .route("/sse", get(mcp_sse_handler))
        .route("/messages", post(mcp_message_handler))
        .route("/tokens", get(list_mcp_tokens).post(create_mcp_token))
        .route("/tokens/:id", axum::routing::delete(delete_mcp_token))
        .route("/", post(mcp_direct_handler))
}

/// GET /api/v1/mcp/sse — Establishes the MCP SSE transport connection.
///
/// Deliberately not `#[utoipa::path]`-annotated: this is an `event-stream`
/// handshake with no JSON request or response body to describe, so an
/// OpenAPI operation would misdescribe it. Allowlisted in
/// `openapi::tests::UNDOCUMENTED` for the same reason.
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
///
/// Deliberately not `#[utoipa::path]`-annotated: the body is a generic
/// JSON-RPC 2.0 envelope (`JsonRpcRequest`/`JsonRpcResponse`) whose actual
/// shape depends on the MCP method being called, so a single schema would
/// misdescribe every method but one. Allowlisted in
/// `openapi::tests::UNDOCUMENTED`.
pub async fn mcp_message_handler(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(_query): Query<McpQuery>,
    Json(req): Json<JsonRpcRequest>,
) -> Result<Json<Option<JsonRpcResponse>>, ApiError> {
    let owner_id = user.owner_id();
    let service = McpService::new(
        &state.services,
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
    );
    let resp = service.handle_request(req).await;
    Ok(Json(resp))
}

/// POST /api/v1/mcp — Direct single-endpoint JSON-RPC handler (no SSE required).
///
/// Deliberately not `#[utoipa::path]`-annotated, for the same reason as
/// [`mcp_message_handler`]: a generic JSON-RPC envelope, not one fixed shape.
/// Allowlisted in `openapi::tests::UNDOCUMENTED`.
pub async fn mcp_direct_handler(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(req): Json<JsonRpcRequest>,
) -> Result<Json<Option<JsonRpcResponse>>, ApiError> {
    let owner_id = user.owner_id();
    let service = McpService::new(
        &state.services,
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
    );
    let resp = service.handle_request(req).await;
    Ok(Json(resp))
}

/// GET /api/v1/mcp/tokens — Lists active personal access tokens for the authenticated user.
#[utoipa::path(
    get,
    tag = "mcp",
    path = "/api/v1/mcp/tokens",
    responses(
        (status = 200, description = "Active personal access tokens for the caller", body = Vec<McpTokenSummary>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_mcp_tokens(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<McpTokenSummary>>, ApiError> {
    let tokens = McpTokenRepository::list_for_account(&state.db.pool, &user.account_id)
        .await
        .map_err(|err| {
            tracing::error!("Failed to list MCP tokens: {:?}", err);
            ApiError::internal("Failed to list MCP tokens")
        })?;

    let summaries = tokens.into_iter().map(McpTokenSummary::from).collect();
    Ok(Json(summaries))
}

/// POST /api/v1/mcp/tokens — Generates a new personal access token and returns client JSON.
#[utoipa::path(
    post,
    tag = "mcp",
    path = "/api/v1/mcp/tokens",
    request_body = CreateMcpTokenRequest,
    responses(
        (status = 200, description = "Token created; the raw token is returned once", body = CreateMcpTokenResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_mcp_token(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateMcpTokenRequest>,
) -> Result<Json<CreateMcpTokenResponse>, ApiError> {
    let name = payload.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("Token name cannot be empty"));
    }
    if name.len() > 100 {
        return Err(ApiError::bad_request(
            "Token name cannot exceed 100 characters",
        ));
    }

    let (raw_token, token) = McpTokenRepository::create(
        &state.db.pool,
        &user.account_id,
        name,
        payload.expires_in_days,
    )
    .await
    .map_err(|err| {
        tracing::error!("Failed to create MCP token: {:?}", err);
        ApiError::internal("Failed to create MCP token")
    })?;

    let endpoint_url = "/api/v1/mcp/sse";
    let claude_config = models::build_claude_config(endpoint_url, &raw_token);
    let cursor_config = models::build_cursor_config(endpoint_url, &raw_token);

    Ok(Json(CreateMcpTokenResponse {
        id: token.id,
        name: token.name,
        token: raw_token,
        prefix: token.prefix,
        created_at: token.created_at,
        expires_at: token.expires_at,
        claude_config,
        cursor_config,
    }))
}

/// DELETE /api/v1/mcp/tokens/:id — Revokes an MCP personal access token.
#[utoipa::path(
    delete,
    tag = "mcp",
    path = "/api/v1/mcp/tokens/{id}",
    params(("id" = String, Path, description = "Token ID")),
    responses(
        (status = 200, description = "Token revoked"),
        (status = 404, description = "Token not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_mcp_token(
    State(state): State<ApiState>,
    user: AuthUser,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let deleted = McpTokenRepository::delete(&state.db.pool, &user.account_id, &id)
        .await
        .map_err(|err| {
            tracing::error!("Failed to delete MCP token: {:?}", err);
            ApiError::internal("Failed to delete MCP token")
        })?;

    if deleted {
        Ok(Json(serde_json::json!({ "success": true })))
    } else {
        Err(ApiError::not_found("Token not found"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_direct_handler_ping() {
        let pool = crate::db::pool::test_pool().await;
        let services = crate::application::AppServices::new(&pool);
        let service = McpService::new(&services, &pool, None, "test_owner");

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

    #[tokio::test]
    async fn test_tokens_api_flow() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        crate::db_execute!(
            &pool,
            "INSERT INTO accounts (id, display_name) VALUES ('acct_test', 'Tester')"
        )
        .unwrap();

        let (raw, token) = McpTokenRepository::create(&pool, "acct_test", "API Test", None)
            .await
            .unwrap();
        assert!(raw.starts_with("mb_mcp_"));

        let list = McpTokenRepository::list_for_account(&pool, "acct_test")
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, token.id);

        let deleted = McpTokenRepository::delete(&pool, "acct_test", &token.id)
            .await
            .unwrap();
        assert!(deleted);
    }
}
