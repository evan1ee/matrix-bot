//! The account's own security log.

use crate::interface::http::account::types::SecurityEvent;
use crate::interface::http::auth::types::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
use axum::{extract::State, Json};

// ------------------------------------------------------------------- audit --

#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/security-events",
    responses((status = 200, body = Vec<SecurityEvent>)),
    security(("session_cookie" = []))
)]
pub async fn list_security_events(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<SecurityEvent>>, ApiError> {
    let events = state
        .services
        .accounts
        .list_security_events(&user.account_id, 50)
        .await?;
    Ok(Json(events.iter().map(SecurityEvent::from).collect()))
}
