//! Linked Matrix identities, and the merge a link can require.

use axum::{extract::State, Json};

use crate::accounts::{
    linking::LINK_CODE_TTL_MINUTES, AccountRepository, LinkRepository, MergePreview,
    MergeRepository, SecurityEventKind, SecurityEventRepository,
};
use crate::api::account::types::*;
use crate::api::auth::extractor::VerifiedUser;
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;

// ------------------------------------------------------------ matrix links --

#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/matrix",
    responses((status = 200, body = Vec<String>)),
    security(("session_cookie" = []))
)]
pub async fn list_links(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<String>>, ApiError> {
    Ok(Json(
        AccountRepository::linked_mxids(state.db.sqlite_pool(), &user.account_id)
            .await
            .map_err(ApiError::from)?,
    ))
}

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/matrix/link",
    responses((status = 200, body = LinkCodeResponse)),
    security(("session_cookie" = []))
)]
pub async fn start_link(
    State(state): State<ApiState>,
    user: VerifiedUser,
) -> Result<Json<LinkCodeResponse>, ApiError> {
    let code = LinkRepository::issue(state.db.sqlite_pool(), &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(LinkCodeResponse {
        code,
        expires_in_seconds: LINK_CODE_TTL_MINUTES * 60,
    }))
}

/// Whether a merge is waiting on this account's confirmation.
///
/// Set when `!link` found the MXID already attached to an unclaimed account
/// holding data — the user is told to come back here, where the counts can
/// actually be shown before they agree to move anything.
#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/matrix/pending-merge",
    responses((status = 200, body = Option<PendingMerge>)),
    security(("session_cookie" = []))
)]
pub async fn pending_merge(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Option<PendingMerge>>, ApiError> {
    let pending = state
        .pending_merges
        .read()
        .await
        .get(&user.account_id)
        .cloned();

    let Some(from_account) = pending else {
        return Ok(Json(None));
    };

    let preview = MergeRepository::preview(state.db.sqlite_pool(), &from_account)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(Some(PendingMerge {
        from_account,
        preview,
    })))
}

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/matrix/confirm-merge",
    request_body = ConfirmMergeRequest,
    responses((status = 200, body = MergePreview)),
    security(("session_cookie" = []))
)]
pub async fn confirm_merge(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Json(payload): Json<ConfirmMergeRequest>,
) -> Result<Json<MergePreview>, ApiError> {
    // The pending entry is the authorisation. Without this check any account
    // could name any other account id and absorb it.
    let authorised = state
        .pending_merges
        .read()
        .await
        .get(&user.account_id)
        .is_some_and(|from| *from == payload.from_account);

    if !authorised {
        return Err(ApiError::not_found("No merge is waiting for confirmation"));
    }

    let outcome = MergeRepository::merge(
        state.db.sqlite_pool(),
        &payload.from_account,
        &user.account_id,
    )
    .await
    .map_err(ApiError::from)?;

    state.pending_merges.write().await.remove(&user.account_id);

    SecurityEventRepository::record(
        state.db.sqlite_pool(),
        &user.account_id,
        SecurityEventKind::AccountsMerged,
        Some(&format!("{} rows", outcome.moved.total())),
    )
    .await;

    Ok(Json(outcome.moved))
}

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/matrix/unlink",
    request_body = UnlinkRequest,
    responses((status = 204, description = "Unlinked")),
    security(("session_cookie" = []))
)]
pub async fn unlink(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Json(payload): Json<UnlinkRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let removed = LinkRepository::unlink(state.db.sqlite_pool(), &user.account_id, &payload.mxid)
        .await
        .map_err(ApiError::from)?;

    if !removed {
        return Err(ApiError::not_found(
            "That Matrix account is not linked here",
        ));
    }

    SecurityEventRepository::record(
        state.db.sqlite_pool(),
        &user.account_id,
        SecurityEventKind::MatrixUnlinked,
        Some(&payload.mxid),
    )
    .await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}
