//! Signed-in devices.

use axum::{extract::State, Json};

use crate::accounts::{SecurityEventKind, SecurityEventRepository, SessionRepository};
use crate::error::ApiError;
use crate::interface::http::account::types::*;
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::types::{ApiState, AuthUser};

// ---------------------------------------------------------------- sessions --

#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/sessions",
    responses((status = 200, body = Vec<SessionSummary>)),
    security(("session_cookie" = []))
)]
pub async fn list_sessions(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<SessionSummary>>, ApiError> {
    let sessions = SessionRepository::list_for_account(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(
        sessions
            .into_iter()
            .map(|s| SessionSummary {
                current: s.id_hash == user.session_id_hash,
                id: s.id_hash,
                created_at: s.created_at,
                last_seen_at: s.last_seen_at,
                user_agent: s.user_agent,
            })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/sessions/revoke-others",
    responses((status = 200, description = "How many were signed out")),
    security(("session_cookie" = []))
)]
pub async fn revoke_other_sessions(
    State(state): State<ApiState>,
    user: VerifiedUser,
) -> Result<Json<u64>, ApiError> {
    let revoked = SessionRepository::revoke_all_for_account(
        &state.db.pool,
        &user.account_id,
        Some(&user.session_id_hash),
    )
    .await
    .map_err(ApiError::from)?;

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::SessionsRevoked,
        Some(&format!("{revoked} session(s)")),
    )
    .await;

    Ok(Json(revoked))
}

#[utoipa::path(
    delete,
    tag = "account",
    path = "/api/v1/account/sessions/{id}",
    params(("id" = String, Path, description = "Session hash ID to revoke")),
    responses((status = 200, description = "Session signed out")),
    security(("session_cookie" = []))
)]
pub async fn revoke_session(
    State(state): State<ApiState>,
    axum::extract::Path(id): axum::extract::Path<String>,
    user: VerifiedUser,
) -> Result<Json<bool>, ApiError> {
    let sessions = SessionRepository::list_for_account(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    if !sessions.iter().any(|s| s.id_hash == id) {
        return Err(ApiError::not_found("Session not found"));
    }

    SessionRepository::revoke(&state.db.pool, &id)
        .await
        .map_err(ApiError::from)?;

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::SessionsRevoked,
        Some("1 session"),
    )
    .await;

    Ok(Json(true))
}
