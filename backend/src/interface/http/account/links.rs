//! Linked Matrix identities (Task 10 generalises these to every channel).

use axum::{extract::State, Json};

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
