//! The account's own security log.

use crate::error::ApiError;
use crate::interface::http::account::types::SecurityEvent;
use crate::interface::http::auth::types::{ApiState, AuthUser};
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

use crate::domain::accounts::AccountId;

/// Records that a `!link` needs the web UI to confirm a merge.
///
/// Held in memory rather than a table: it is a handoff between two halves of
/// one flow, valid only until the user answers, and a stale entry should
/// evaporate on restart rather than linger as a standing authorisation to
/// absorb an account.
pub async fn note_pending_merge(state: &ApiState, into: &AccountId, from: &AccountId) {
    state
        .pending_merges
        .write()
        .await
        .insert(into.to_string(), from.to_string());
}
