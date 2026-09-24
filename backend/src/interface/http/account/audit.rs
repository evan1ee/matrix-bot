//! The account's own security log.

use axum::{extract::State, Json};

use crate::accounts::security_events::SecurityEvent;
use crate::accounts::SecurityEventRepository;
use crate::error::ApiError;
use crate::interface::http::auth::types::{ApiState, AuthUser};

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
    Ok(Json(
        SecurityEventRepository::list(&state.db.pool, &user.account_id, 50)
            .await
            .map_err(ApiError::from)?,
    ))
}

/// Records that a `!link` needs the web UI to confirm a merge.
///
/// Held in memory rather than a table: it is a handoff between two halves of
/// one flow, valid only until the user answers, and a stale entry should
/// evaporate on restart rather than linger as a standing authorisation to
/// absorb an account.
pub async fn note_pending_merge(state: &ApiState, into: &str, from: &str) {
    state
        .pending_merges
        .write()
        .await
        .insert(into.to_string(), from.to_string());
}
