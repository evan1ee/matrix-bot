//! The account's own security log.

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::interface::http::auth::types::{ApiState, AuthUser};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SecurityEvent {
    pub id: i64,
    pub kind: String,
    pub detail: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::accounts::SecurityEvent> for SecurityEvent {
    fn from(ev: crate::domain::accounts::SecurityEvent) -> Self {
        Self {
            id: ev.id,
            kind: ev.kind,
            detail: ev.detail,
            created_at: ev.created_at,
        }
    }
}

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
    Ok(Json(events.into_iter().map(SecurityEvent::from).collect()))
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
