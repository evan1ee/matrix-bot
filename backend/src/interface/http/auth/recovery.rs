//! Endpoint for redeeming account recovery codes to regain access.

use axum::{
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use chrono::{Duration, Utc};
use serde::Deserialize;
use std::net::SocketAddr;
use utoipa::ToSchema;

use crate::accounts::RECOVERY_RESTRICTION_HOURS;
use crate::domain::accounts::SecurityEventKind;
use crate::error::ApiError;
use crate::interface::http::auth::types::ApiState;
use crate::interface::http::auth::webauthn::{with_session_cookie, SessionResponse};
use crate::interface::http::rate_limit::check_recovery_limit;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RedeemRecoveryRequest {
    pub code: String,
}

fn is_trusted_private_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        std::net::IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// Helper to extract client IP for rate-limiting.
///
/// By default, rate-limiting keys on the peer socket address from `connect_info`.
/// Forwarded headers (`X-Forwarded-For`, `X-Real-IP`) are ONLY trusted if
/// `trust_forwarded` is enabled AND the connecting peer is a private/loopback reverse proxy.
fn extract_client_ip(
    connect_info: Option<&ConnectInfo<SocketAddr>>,
    headers: &HeaderMap,
    trust_forwarded: bool,
) -> String {
    if let Some(ConnectInfo(addr)) = connect_info {
        let peer_ip = addr.ip();

        if trust_forwarded && is_trusted_private_ip(peer_ip) {
            if let Some(forwarded) = headers
                .get("x-forwarded-for")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.split(',').next())
                .map(|s| s.trim())
            {
                if let Ok(parsed) = forwarded.parse::<std::net::IpAddr>() {
                    return parsed.to_string();
                }
            }
            if let Some(real_ip) = headers
                .get("x-real-ip")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.trim())
            {
                if let Ok(parsed) = real_ip.parse::<std::net::IpAddr>() {
                    return parsed.to_string();
                }
            }
        }
        return peer_ip.to_string();
    }

    "127.0.0.1".to_string()
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
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    Json(payload): Json<RedeemRecoveryRequest>,
) -> Result<Response, ApiError> {
    let client_ip = extract_client_ip(connect_info.as_ref(), &headers, state.trust_forwarded_for);
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
    let mut resolved_account_id = state
        .services
        .accounts
        .redeem_recovery_code(code)
        .await
        .map_err(|e| ApiError::internal(format!("Database error during recovery check: {e}")))?;

    // 2. If not found in static codes, try 24-hour matured recovery requests
    if resolved_account_id.is_none() {
        if let Ok(account_id) = crate::interface::chat::account::redeem_matured_recovery(
            &state.services.accounts,
            &state.db,
            None,
            code,
        )
        .await
        {
            resolved_account_id = Some(account_id);
        }
    }

    let account_id = resolved_account_id
        .ok_or_else(|| ApiError::unauthorized("Invalid or expired recovery code"))?;

    let account = state
        .services
        .accounts
        .find(&account_id)
        .await
        .map_err(|err| ApiError::internal(format!("Could not read account: {err}")))?
        .ok_or_else(|| ApiError::unauthorized("Account not found"))?;

    // Revoke any previous session cookie if provided
    if let Some(cookies) = headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(old_token) =
            crate::accounts::principal::cookie_value(cookies, crate::accounts::session::COOKIE_NAME)
        {
            let old_id_hash = crate::accounts::session::hash_token(old_token);
            let _ = state
                .services
                .accounts
                .revoke_session(&account_id, &old_id_hash)
                .await;
        }
    }

    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok());

    // Mint a restricted session (restricted to registering a new passkey)
    let restricted_until = Utc::now() + Duration::hours(RECOVERY_RESTRICTION_HOURS);
    let token = state
        .services
        .accounts
        .create_session(
            &account_id,
            user_agent,
            Some(&client_ip),
            Some(restricted_until),
        )
        .await
        .map_err(|err| ApiError::internal(format!("Could not open restricted session: {err}")))?;

    // Record security event
    state
        .services
        .accounts
        .record_security_event(&account_id, SecurityEventKind::RecoveryCompleted, None)
        .await;

    let body = Json(SessionResponse {
        account_id: account_id.to_string(),
        display_name: account.display_name,
    });

    Ok(with_session_cookie(body, &token, state.cookie_secure))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn test_recovery_redemption_flow() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool.clone() },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let account_id = state
            .services
            .accounts
            .create("Recovery User", crate::domain::accounts::STATUS_ACTIVE)
            .await
            .unwrap();

        let codes = state
            .services
            .accounts
            .issue_recovery_codes(&account_id)
            .await
            .unwrap();
        let valid_code = &codes[0];

        // 1. Valid code succeeds
        let res = redeem_recovery(
            State(state.clone()),
            None,
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
            None,
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
            None,
            HeaderMap::new(),
            Json(RedeemRecoveryRequest {
                code: "invalid-code-here".to_string(),
            }),
        )
        .await;
        assert!(res_bad.is_err());
    }

    #[tokio::test]
    async fn test_recovery_rate_limit_cannot_be_bypassed_with_spoofed_headers() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let peer_addr: SocketAddr = "198.51.100.99:54321".parse().unwrap();

        // Send 5 invalid attempts with rotating X-Forwarded-For headers
        for i in 1..=5 {
            let mut headers = HeaderMap::new();
            headers.insert("x-forwarded-for", format!("10.0.0.{}", i).parse().unwrap());
            let res = redeem_recovery(
                State(state.clone()),
                Some(ConnectInfo(peer_addr)),
                headers,
                Json(RedeemRecoveryRequest {
                    code: format!("wrong-code-{}", i),
                }),
            )
            .await;
            // First 5 attempts fail with unauthorized/not found (401)
            let Err(err) = res else {
                panic!("expected failure")
            };
            assert_ne!(err.status, StatusCode::TOO_MANY_REQUESTS);
        }

        // 6th attempt with yet another spoofed X-Forwarded-For must be blocked by rate limiter (429)
        // because the rate limiter keys on the peer socket address (198.51.100.99), not the spoofed header!
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "10.0.0.99".parse().unwrap());
        let res_limited = redeem_recovery(
            State(state.clone()),
            Some(ConnectInfo(peer_addr)),
            headers,
            Json(RedeemRecoveryRequest {
                code: "wrong-code-6".to_string(),
            }),
        )
        .await;

        let Err(err) = res_limited else {
            panic!("expected rate limit error")
        };
        assert_eq!(err.status, StatusCode::TOO_MANY_REQUESTS);
    }
}
