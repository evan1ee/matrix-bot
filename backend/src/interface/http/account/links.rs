//! Linked Matrix identities, and the merge a link can require.

use axum::{extract::State, Json};

use crate::domain::accounts::{AccountId, LINK_CODE_TTL_MINUTES};
use crate::domain::channels::Channel;
use crate::error::ApiError;
use crate::interface::http::account::types::*;
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::types::{ApiState, AuthUser};

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
        state
            .services
            .identity
            .user_ids(&user.account_id, Channel::Matrix)
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
    let code = state
        .services
        .accounts
        .issue_link_challenge(&user.account_id)
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
        .get(&user.account_id.to_string())
        .cloned();

    let Some(from_account) = pending else {
        return Ok(Json(None));
    };

    let from_id: AccountId = from_account
        .parse()
        .map_err(|_| ApiError::internal("pending merge holds a malformed account id"))?;
    let domain_preview = state
        .services
        .accounts
        .preview_merge(&from_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(Some(PendingMerge {
        from_account,
        preview: MergePreview::from(domain_preview),
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
        .get(&user.account_id.to_string())
        .is_some_and(|from| *from == payload.from_account);

    if !authorised {
        return Err(ApiError::not_found("No merge is waiting for confirmation"));
    }

    let from_id: AccountId = payload
        .from_account
        .parse()
        .map_err(|_| ApiError::bad_request("from_account is not a valid account id"))?;
    let outcome = state
        .services
        .accounts
        .merge(&from_id, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    state
        .pending_merges
        .write()
        .await
        .remove(&user.account_id.to_string());

    Ok(Json(MergePreview::from(outcome.moved)))
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
    let removed = state
        .services
        .identity
        .unlink(&user.account_id, Channel::Matrix, &payload.mxid)
        .await
        .map_err(ApiError::from)?;

    if !removed {
        return Err(ApiError::not_found(
            "That Matrix account is not linked here",
        ));
    }

    Ok(axum::http::StatusCode::NO_CONTENT)
}
