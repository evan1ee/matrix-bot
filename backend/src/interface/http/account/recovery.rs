//! Recovery codes, and stopping a recovery someone else started.

use axum::{extract::State, Json};

use crate::error::ApiError;
use crate::interface::http::account::types::*;
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::types::{ApiState, AuthUser};

// ---------------------------------------------------------------- recovery --

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/recovery-codes",
    responses((status = 200, body = RecoveryCodesResponse)),
    security(("session_cookie" = []))
)]
pub async fn regenerate_recovery_codes(
    State(state): State<ApiState>,
    user: VerifiedUser,
) -> Result<Json<RecoveryCodesResponse>, ApiError> {
    let codes = state
        .services
        .accounts
        .issue_recovery_codes(&user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(RecoveryCodesResponse { codes }))
}

#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/recovery-status",
    responses((status = 200, description = "Codes left, and any recovery counting down")),
    security(("session_cookie" = []))
)]
pub async fn recovery_status(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<serde_json::Value>, ApiError> {
    let remaining = state
        .services
        .accounts
        .remaining_recovery_codes(&user.account_id)
        .await
        .unwrap_or(0);
    let pending = state
        .services
        .accounts
        .pending_recovery(&user.account_id)
        .await
        .unwrap_or(None);

    Ok(Json(serde_json::json!({
        "codes_remaining": remaining,
        "recovery_valid_after": pending,
    })))
}

/// Stops a recovery someone started from a linked Matrix account.
///
/// Deliberately takes [`AuthUser`], not [`VerifiedUser`]. Everything else here
/// is gated on a fresh passkey, but this is the *defence* against a recovery
/// nobody asked for — demanding a passkey to cancel would be exactly backwards
/// when the person cancelling may be racing a 24-hour clock on their phone.
#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/recovery/cancel",
    responses((status = 200, description = "How many requests were cancelled")),
    security(("session_cookie" = []))
)]
pub async fn cancel_recovery(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<u64>, ApiError> {
    let cancelled = state
        .services
        .accounts
        .cancel_recovery(&user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(cancelled))
}
