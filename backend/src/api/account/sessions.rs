//! Signed-in devices.

use axum::{
    extract::State,
    Json,
};

use crate::accounts::{
    SecurityEventKind, SecurityEventRepository, SessionRepository,
};
use crate::api::account::types::*;
use crate::api::auth::extractor::VerifiedUser;
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;

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
