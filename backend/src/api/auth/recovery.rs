//! Endpoint for redeeming account recovery codes to regain access.

use axum::{extract::State, http::HeaderMap, response::Response, Json};
use chrono::{Duration, Utc};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::accounts::{
    AccountRepository, RecoveryRepository, SecurityEventKind, SecurityEventRepository,
    SessionRepository, RECOVERY_RESTRICTION_HOURS,
};
use crate::api::auth::types::ApiState;
use crate::api::auth::webauthn::{with_session_cookie, SessionResponse};
use crate::api::rate_limit::check_recovery_limit;
use crate::error::ApiError;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RedeemRecoveryRequest {
    pub code: String,
}

/// Helper to extract client IP for rate-limiting.
fn extract_client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim().to_string())
        })
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

#[utoipa::path(
    post,
    tag = "auth",
    path = "/api/v1/auth/recovery/redeem",
    request_body = RedeemRecoveryRequest,
    responses(
        (status = 200, description = "Recovery code accepted; restricted session opened", body = SessionResponse),
        (status = 401, description = "Invalid or expired recovery code", body = ApiErrorResponse),
        (status = 429, description = "Rate limit exceeded", body = ApiErrorResponse)
    )
)]
pub async fn redeem_recovery(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<RedeemRecoveryRequest>,
) -> Result<Response, ApiError> {
    let client_ip = extract_client_ip(&headers);
    if !check_recovery_limit(&client_ip) {
        return Err(ApiError::too_many_requests(
            "Too many recovery attempts. Please wait 15 minutes before trying again.",
        ));
    }

    let code = payload.code.trim();
    if code.is_empty() {
        return Err(ApiError::bad_request("Recovery code cannot be empty"));
    }

    // 1. Try static backup recovery codes
    let mut resolved_account_id = RecoveryRepository::redeem(&state.db.pool, code)
        .await
        .map_err(|e| ApiError::internal(format!("Database error during recovery check: {e}")))?;

    // 2. If not found in static codes, try 24-hour matured recovery requests
    if resolved_account_id.is_none() {
        if let Ok(account_id) =
            crate::modules::account::redeem_matured_recovery(&state.db, None, code).await
        {
            resolved_account_id = Some(account_id);
        }
    }

    let account_id = resolved_account_id
        .ok_or_else(|| ApiError::unauthorized("Invalid or expired recovery code"))?;

    let account = AccountRepository::find(&state.db.pool, &account_id)
        .await
        .map_err(|err| ApiError::internal(format!("Could not read account: {err}")))?
        .ok_or_else(|| ApiError::unauthorized("Account not found"))?;

    // Revoke any previous session cookie if provided
    if let Some(cookies) = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(old_token) = crate::api::auth::extractor::cookie_value(
            cookies,
            crate::accounts::session::COOKIE_NAME,
        ) {
            let old_id_hash = crate::accounts::session::hash_token(old_token);
            let _ = SessionRepository::revoke(&state.db.pool, &old_id_hash).await;
        }
    }

    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok());

    // Mint a restricted session (restricted to registering a new passkey)
    let restricted_until = Utc::now() + Duration::hours(RECOVERY_RESTRICTION_HOURS);
    let token = SessionRepository::create(
        &state.db.pool,
        &account_id,
        user_agent,
        Some(&client_ip),
        Some(restricted_until),
    )
    .await
    .map_err(|err| ApiError::internal(format!("Could not open restricted session: {err}")))?;

    // Record security event
    SecurityEventRepository::record(
        &state.db.pool,
        &account_id,
        SecurityEventKind::RecoveryCompleted,
        None,
    )
    .await;

    let body = Json(SessionResponse {
        account_id,
        display_name: account.display_name,
    });

    Ok(with_session_cookie(body, &token, state.cookie_secure))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::AccountRepository;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_recovery_redemption_flow() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool: pool.clone() },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let account_id =
            AccountRepository::create(&pool, "Recovery User", crate::accounts::STATUS_ACTIVE)
                .await
                .unwrap();

        let codes = RecoveryRepository::issue_codes(&pool, &account_id)
            .await
            .unwrap();
        let valid_code = &codes[0];

        // 1. Valid code succeeds
        let res = redeem_recovery(
            State(state.clone()),
            HeaderMap::new(),
            Json(RedeemRecoveryRequest {
                code: valid_code.clone(),
            }),
        )
        .await;
        assert!(res.is_ok());

        // 2. Code is single-use, second redemption fails
        let res_retry = redeem_recovery(
            State(state.clone()),
            HeaderMap::new(),
            Json(RedeemRecoveryRequest {
                code: valid_code.clone(),
            }),
        )
        .await;
        assert!(res_retry.is_err());

        // 3. Invalid code fails
        let res_bad = redeem_recovery(
            State(state.clone()),
            HeaderMap::new(),
            Json(RedeemRecoveryRequest {
                code: "invalid-code-here".to_string(),
            }),
        )
        .await;
        assert!(res_bad.is_err());
    }
}
