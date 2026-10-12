//! Registered passkeys: list, rename, remove.

use axum::{
    extract::{Path, State},
    Json,
};

use crate::domain::accounts::DeleteOutcome;
use crate::interface::http::account::types::*;
use crate::interface::http::auth::extractor::VerifiedUser;
use crate::interface::http::auth::types::{ApiState, AuthUser, Ceremony};
use crate::interface::http::auth::webauthn::CeremonyStart;
use crate::interface::http::error::ApiError;

// ---------------------------------------------------------------- passkeys --

#[utoipa::path(
    post,
    tag = "account",
    path = "/api/v1/account/passkeys/add/start",
    responses((status = 200, body = CeremonyStart)),
    security(("session_cookie" = []))
)]
pub async fn add_passkey_start(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<CeremonyStart>, ApiError> {
    let now = chrono::Utc::now();
    let active_restriction = user.restricted_until.filter(|&until| now < until);
    if active_restriction.is_none() && !user.may_perform_sensitive_action(now) {
        return Err(ApiError::forbidden("Confirm with your passkey to continue"));
    }

    let handle = user.account_id.to_handle();

    let stored = state
        .services
        .accounts
        .list_passkeys(&user.account_id)
        .await
        .unwrap_or_default();
    let exclude: Vec<webauthn_rs::prelude::CredentialID> = stored
        .iter()
        .filter_map(crate::application::accounts::AccountService::parse_passkey)
        .map(|p| p.cred_id().clone())
        .collect();
    let exclude_opt = if exclude.is_empty() {
        None
    } else {
        Some(exclude)
    };

    let (options, reg_state) = state
        .webauthn
        .start_passkey_registration(handle, &user.display_name, &user.display_name, exclude_opt)
        .map_err(|err| {
            tracing::error!("Could not start passkey registration: {:?}", err);
            ApiError::internal("Could not start passkey registration")
        })?;

    let ceremony_id = state
        .stash_ceremony(Ceremony::Registration {
            account_id: user.account_id,
            display_name: user.display_name,
            restricted_until: active_restriction,
            state: reg_state,
        })
        .await?;

    Ok(Json(CeremonyStart {
        ceremony_id,
        options: serde_json::to_value(&options)
            .map_err(|err| ApiError::internal(err.to_string()))?,
    }))
}

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
    let stored = state
        .services
        .accounts
        .list_passkeys(&user.account_id)
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

    let renamed = state
        .services
        .accounts
        .rename_passkey(&user.account_id, &id, label)
        .await
        .map_err(ApiError::from)?;

    if !renamed {
        return Err(ApiError::not_found("No such passkey"));
    }

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
    match state
        .services
        .accounts
        .delete_passkey(&user.account_id, &id)
        .await
        .map_err(ApiError::from)?
    {
        DeleteOutcome::Removed => Ok(axum::http::StatusCode::NO_CONTENT),
        DeleteOutcome::NotFound => Err(ApiError::not_found("No such passkey")),
        DeleteOutcome::WouldLockOut => Err(ApiError::conflict(
            "That is your only passkey. Add another before removing this one.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use chrono::{Duration, Utc};

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_add_passkey_start_rejects_stale_session_and_preserves_recovery_restriction() {
        let state = test_state().await;
        let account_id = state
            .services
            .accounts
            .create("evan", STATUS_ACTIVE)
            .await
            .unwrap();

        // 1. Stale unverified session is rejected with 403
        let stale_user = AuthUser {
            last_verified_at: Utc::now() - Duration::minutes(10),
            restricted_until: None,
            ..AuthUser::for_account(account_id)
        };
        let err = add_passkey_start(State(state.clone()), stale_user)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::FORBIDDEN);

        // 2. Recovery-restricted session is allowed to start passkey registration,
        // and its restricted_until is carried into the ceremony and the resulting session.
        let until = Utc::now() + Duration::hours(72);
        let recovery_user = AuthUser {
            last_verified_at: chrono::DateTime::<Utc>::UNIX_EPOCH,
            restricted_until: Some(until),
            ..AuthUser::for_account(account_id)
        };
        let started = add_passkey_start(State(state.clone()), recovery_user)
            .await
            .expect("recovery-restricted session may start passkey registration");
        let Some(Ceremony::Registration {
            restricted_until, ..
        }) = state.take_ceremony(&started.ceremony_id).await
        else {
            panic!("expected registration ceremony");
        };
        assert_eq!(restricted_until, Some(until));

        // 3. Issuing a session with that restriction keeps restricted_until and does not satisfy step-up.
        let resp = crate::interface::http::auth::webauthn::issue_session_with_restriction(
            &state,
            &account_id,
            "evan",
            &axum::http::HeaderMap::new(),
            restricted_until,
        )
        .await
        .unwrap();
        let set_cookie = resp
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        let token =
            crate::domain::accounts::cookie_value(set_cookie, crate::domain::accounts::COOKIE_NAME)
                .unwrap();
        let session = state
            .services
            .accounts
            .resolve_session(token)
            .await
            .unwrap()
            .unwrap();
        assert!(session.is_restricted(Utc::now()));
        assert!(!session.is_recently_verified(Utc::now()));
    }
}
