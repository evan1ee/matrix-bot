//! Endpoint for redeeming account recovery codes to regain access.

use axum::{
    extract::{ConnectInfo, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use chrono::Utc;
use serde::Deserialize;
use std::net::SocketAddr;
use utoipa::ToSchema;

use crate::domain::accounts::{hash_token, restriction_expiry, COOKIE_NAME};
use crate::interface::http::auth::types::ApiState;
use crate::interface::http::auth::webauthn::{with_session_cookie, SessionResponse};
use crate::interface::http::error::ApiError;
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
pub(crate) fn extract_client_ip(
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
        match crate::interface::chat::account::redeem_matured_recovery(&state.services, None, code).await {
            Ok(account_id) => {
                resolved_account_id = Some(account_id);
            }
            Err(crate::interface::chat::account::RecoveryRejection::TooEarly)
            | Err(crate::interface::chat::account::RecoveryRejection::Unavailable) => {}
            Err(crate::interface::chat::account::RecoveryRejection::Database(err)) => {
                return Err(ApiError::internal(format!("Database error during recovery check: {err}")));
            }
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
        if let Some(old_token) = crate::domain::accounts::cookie_value(cookies, COOKIE_NAME) {
            let old_id_hash = hash_token(old_token);
            // Whatever session the browser presents must die. The scoped,
            // audited revoke covers the account's own session; anything
            // else (another account's cookie) falls back to the unscoped,
            // unaudited hash revoke. Errors are ignored: the response
            // replaces the cookie either way.
            let accounts = &state.services.accounts;
            if !matches!(
                accounts.revoke_session(&account_id, &old_id_hash).await,
                Ok(true)
            ) {
                let _ = accounts.end_session(&old_id_hash).await;
            }
        }
    }

    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok());

    // Mint a restricted session (restricted to registering a new passkey)
    let restricted_until = restriction_expiry(Utc::now());
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

    let body = Json(SessionResponse {
        account_id: account_id.to_string(),
        display_name: account.display_name,
    });

    Ok(with_session_cookie(body, &token, state.cookie_secure))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use chrono::Duration;

    #[tokio::test]
    async fn test_recovery_redemption_flow() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool: pool.clone() },
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

    async fn state_with_account() -> (ApiState, crate::domain::accounts::AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
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
        (state, account_id)
    }

    async fn audit_kinds(
        state: &ApiState,
        account_id: &crate::domain::accounts::AccountId,
    ) -> Vec<String> {
        let mut kinds: Vec<String> = state
            .services
            .accounts
            .list_security_events(account_id, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        kinds.sort();
        kinds
    }

    fn redeem_request(code: &str) -> Json<RedeemRecoveryRequest> {
        Json(RedeemRecoveryRequest {
            code: code.to_string(),
        })
    }

    #[tokio::test]
    async fn redeeming_a_code_records_exactly_the_events_it_did_before() {
        let (state, account_id) = state_with_account().await;
        let codes = state
            .services
            .accounts
            .issue_recovery_codes(&account_id)
            .await
            .unwrap();

        redeem_recovery(
            State(state.clone()),
            None,
            HeaderMap::new(),
            redeem_request(&codes[0]),
        )
        .await
        .unwrap();

        assert_eq!(
            audit_kinds(&state, &account_id).await,
            vec![
                "recovery_code_used",
                "recovery_codes_issued",
                "recovery_completed"
            ]
        );
    }

    /// A matured `!recover` code used through the web is one recovery, so it
    /// leaves one `recovery_completed` row, whichever layer announced it.
    #[tokio::test]
    async fn a_matured_request_records_recovery_completed_once() {
        let (state, account_id) = state_with_account().await;
        let (code, _) = state
            .services
            .accounts
            .request_recovery(&account_id, None)
            .await
            .unwrap();
        crate::infrastructure::postgres::repositories::accounts::recovery::set_valid_after(
            &state.db.pool,
            &account_id,
            Utc::now() - Duration::minutes(1),
        )
        .await
        .unwrap();

        redeem_recovery(
            State(state.clone()),
            None,
            HeaderMap::new(),
            redeem_request(&code),
        )
        .await
        .unwrap();

        assert_eq!(
            audit_kinds(&state, &account_id).await,
            vec!["recovery_completed", "recovery_requested"]
        );
    }

    #[tokio::test]
    async fn a_recovery_session_is_restricted_for_72_hours() {
        let (state, account_id) = state_with_account().await;
        let codes = state
            .services
            .accounts
            .issue_recovery_codes(&account_id)
            .await
            .unwrap();
        let before = Utc::now();

        let response = redeem_recovery(
            State(state.clone()),
            None,
            HeaderMap::new(),
            redeem_request(&codes[0]),
        )
        .await
        .unwrap();

        let set_cookie = response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap();
        let token = crate::domain::accounts::cookie_value(set_cookie, COOKIE_NAME).unwrap();
        let session = state
            .services
            .accounts
            .resolve_session(token)
            .await
            .unwrap()
            .unwrap();
        let expected = crate::domain::accounts::restriction_expiry(before);
        let restricted_until = session
            .restricted_until
            .expect("session must be restricted");
        assert!(
            (restricted_until - expected).num_seconds().abs() <= 5,
            "restricted_until {restricted_until} should be ~{expected}"
        );
    }

    #[tokio::test]
    async fn test_recovery_rate_limit_cannot_be_bypassed_with_spoofed_headers() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
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

    struct Fixture {
        state: ApiState,
        account: crate::domain::accounts::AccountId,
        code: String,
    }

    async fn fixture() -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let accounts = &state.services.accounts;
        let account = accounts
            .create("Recoverer", crate::domain::accounts::STATUS_ACTIVE)
            .await
            .unwrap();
        let code = accounts
            .issue_recovery_codes(&account)
            .await
            .unwrap()
            .remove(0);
        Fixture {
            state,
            account,
            code,
        }
    }

    /// Redeems `f.code` from a distinct client IP (the recovery limiter is
    /// process-wide) presenting `cookie` as the browser's old session.
    async fn redeem_with_cookie(f: &Fixture, ip_last_octet: u8, cookie: &str) {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::COOKIE,
            format!("{COOKIE_NAME}={cookie}").parse().unwrap(),
        );
        let peer: SocketAddr = format!("203.0.113.{ip_last_octet}:4000").parse().unwrap();
        redeem_recovery(
            State(f.state.clone()),
            Some(ConnectInfo(peer)),
            headers,
            Json(RedeemRecoveryRequest {
                code: f.code.clone(),
            }),
        )
        .await
        .expect("recovery succeeds");
    }

    async fn is_live(f: &Fixture, token: &str) -> bool {
        f.state
            .services
            .accounts
            .resolve_credential(&crate::domain::accounts::Credential::Cookie(token))
            .await
            .unwrap()
            .is_some()
    }

    async fn revoke_rows(f: &Fixture, account: &crate::domain::accounts::AccountId) -> usize {
        f.state
            .services
            .accounts
            .list_security_events(account, 100)
            .await
            .unwrap()
            .iter()
            .filter(|e| e.kind == "sessions_revoked")
            .count()
    }

    /// Given the recovering account's own live cookie, When it redeems a
    /// code, Then that session is revoked with exactly one audit row.
    #[tokio::test]
    async fn recovery_revokes_the_accounts_own_cookie_with_one_audit_row() {
        let f = fixture().await;
        let accounts = &f.state.services.accounts;
        let token = accounts
            .create_session(&f.account, None, None, None)
            .await
            .unwrap();

        redeem_with_cookie(&f, 11, &token).await;

        assert!(!is_live(&f, &token).await);
        assert_eq!(revoke_rows(&f, &f.account).await, 1);
    }

    /// Given a cookie belonging to a DIFFERENT account, When a recovery is
    /// redeemed, Then that session is still killed and neither account gets
    /// an audit row.
    #[tokio::test]
    async fn recovery_still_revokes_a_foreign_cookie_without_auditing() {
        let f = fixture().await;
        let accounts = &f.state.services.accounts;
        let other = accounts
            .create("Other", crate::domain::accounts::STATUS_ACTIVE)
            .await
            .unwrap();
        let foreign = accounts
            .create_session(&other, None, None, None)
            .await
            .unwrap();

        redeem_with_cookie(&f, 12, &foreign).await;

        assert!(!is_live(&f, &foreign).await, "a presented session dies");
        assert_eq!(revoke_rows(&f, &f.account).await, 0);
        assert_eq!(revoke_rows(&f, &other).await, 0);
    }

    /// Given an unknown cookie, When a recovery is redeemed, Then nothing
    /// changes: no audit row, and the account's other live session survives.
    #[tokio::test]
    async fn recovery_with_an_unknown_cookie_changes_nothing() {
        let f = fixture().await;
        let accounts = &f.state.services.accounts;
        let bystander = accounts
            .create_session(&f.account, None, None, None)
            .await
            .unwrap();

        redeem_with_cookie(&f, 13, "no-such-token").await;

        assert!(is_live(&f, &bystander).await);
        assert_eq!(revoke_rows(&f, &f.account).await, 0);
    }
}
