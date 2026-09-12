//! Registered passkeys: list, rename, remove.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::accounts::{
    credentials::DeleteOutcome, CredentialRepository, SecurityEventKind, SecurityEventRepository,
};
use crate::api::account::types::*;
use crate::api::auth::extractor::VerifiedUser;
use crate::api::auth::types::{ApiState, AuthUser};
use crate::error::ApiError;

// ---------------------------------------------------------------- passkeys --

#[utoipa::path(
    get,
    tag = "account",
    path = "/api/v1/account/passkeys",
    responses((status = 200, body = Vec<PasskeySummary>)),
    security(("session_cookie" = []))
)]
pub async fn list_passkeys(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<PasskeySummary>>, ApiError> {
    let stored = CredentialRepository::list_for_account(&state.db.pool, &user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(
        stored
            .into_iter()
            .map(|c| PasskeySummary {
                id: c.id,
                label: c.label,
                created_at: c.created_at,
                last_used_at: c.last_used_at,
            })
            .collect(),
    ))
}

#[utoipa::path(
    put,
    tag = "account",
    path = "/api/v1/account/passkeys/{id}",
    request_body = RenamePasskeyRequest,
    responses((status = 204, description = "Renamed")),
    security(("session_cookie" = []))
)]
pub async fn rename_passkey(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Path(id): Path<String>,
    Json(payload): Json<RenamePasskeyRequest>,
) -> Result<axum::http::StatusCode, ApiError> {
    let label = payload.label.trim();
    if label.is_empty() || label.len() > 64 {
        return Err(ApiError::bad_request(
            "A passkey name must be 1-64 characters",
        ));
    }

    let renamed = CredentialRepository::rename(&state.db.pool, &user.account_id, &id, label)
        .await
        .map_err(ApiError::from)?;

    if !renamed {
        return Err(ApiError::not_found("No such passkey"));
    }

    SecurityEventRepository::record(
        &state.db.pool,
        &user.account_id,
        SecurityEventKind::PasskeyRenamed,
        Some(label),
    )
    .await;

    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    delete,
    tag = "account",
    path = "/api/v1/account/passkeys/{id}",
    responses(
        (status = 204, description = "Removed"),
        (status = 409, description = "Would lock the account out", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_passkey(
    State(state): State<ApiState>,
    user: VerifiedUser,
    Path(id): Path<String>,
) -> Result<axum::http::StatusCode, ApiError> {
    match CredentialRepository::delete(&state.db.pool, &user.account_id, &id)
        .await
        .map_err(ApiError::from)?
    {
        DeleteOutcome::Removed => {
            SecurityEventRepository::record(
                &state.db.pool,
                &user.account_id,
                SecurityEventKind::PasskeyRemoved,
                None,
            )
            .await;
            Ok(axum::http::StatusCode::NO_CONTENT)
        }
        DeleteOutcome::NotFound => Err(ApiError::not_found("No such passkey")),
        DeleteOutcome::WouldLockOut => Err(ApiError::conflict(
            "That is your only passkey. Add another before removing this one.",
        )),
    }
}
