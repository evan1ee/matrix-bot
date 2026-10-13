use axum::{
    extract::State,
    http::{header, HeaderMap},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::application::identity::Resolution;
use crate::domain::channels::PlatformIdentity;
use crate::interface::http::auth::types::ApiState;
use crate::interface::http::auth::webauthn::with_session_cookie;
use crate::interface::http::error::ApiError;
#[allow(
    unused_imports,
    reason = "named only inside #[utoipa::path] attributes, where it fixes the OpenAPI schema name; a path-qualified body would rename it"
)]
use crate::interface::http::error::ApiErrorResponse;

/// The bot's access token, if the bot has one.
///
/// Reads the shared slot the bot bootstrap populates, falling back to the token
/// the bot cached on disk at last login. It deliberately does **not** log in:
/// authenticating as the bot is the bot's job, and doing it from inside a
/// request handler meant a single HTTP call could trigger a password login and
/// rewrite `data/matrix_token.json` as a side effect. If the bot has not logged
/// in, callers get `None` and degrade accordingly.
pub async fn get_bot_matrix_access_token(state: &ApiState) -> Option<String> {
    if let Some(tok) = state.bot_access_token.read().await.clone() {
        return Some(tok);
    }

    let file_tok = std::fs::read_to_string("data/matrix_token.json").ok()?;
    let trimmed = file_tok.trim().to_string();
    if trimmed.is_empty() {
        return None;
    }

    *state.bot_access_token.write().await = Some(trimmed.clone());
    Some(trimmed)
}

/// Reads `m.homeserver.base_url` from the configured domain's `.well-known`.
///
/// Done explicitly rather than by inspecting a freshly built SDK client: that
/// builder falls back to a client pointed at the *undelegated* URL when
/// discovery fails, which is indistinguishable from success at the call site.
async fn discover_homeserver(state: &ApiState) -> Option<String> {
    #[derive(Deserialize)]
    struct HomeserverInfo {
        base_url: String,
    }
    #[derive(Deserialize)]
    struct WellKnown {
        #[serde(rename = "m.homeserver")]
        homeserver: HomeserverInfo,
    }

    let base = state.homeserver.trim_end_matches('/');
    let res = state
        .http_client
        .get(format!("{}/.well-known/matrix/client", base))
        .send()
        .await
        .ok()?;

    if !res.status().is_success() {
        return None;
    }

    let well_known: WellKnown = res.json().await.ok()?;
    Some(
        well_known
            .homeserver
            .base_url
            .trim_end_matches('/')
            .to_string(),
    )
}

/// The homeserver base URL for client-API calls, resolved once and reused.
///
/// A delegating domain (`MATRIX_HOMESERVER=https://mbm.one`) does not serve the
/// client API itself — requests there 404. Resolution therefore prefers the
/// bot's live client, whose homeserver is already known-good, then explicit
/// `.well-known` discovery. The configured value is only ever a last resort and
/// is **not** cached, so a transient outage cannot poison the cache for the rest
/// of the process's life.
pub async fn resolved_homeserver(state: &ApiState) -> String {
    if let Some(url) = state.resolved_homeserver.read().await.clone() {
        return url;
    }

    if let Some(client) = state.bot_client.read().await.clone() {
        let url = client
            .homeserver()
            .to_string()
            .trim_end_matches('/')
            .to_string();
        *state.resolved_homeserver.write().await = Some(url.clone());
        return url;
    }

    if let Some(url) = discover_homeserver(state).await {
        *state.resolved_homeserver.write().await = Some(url.clone());
        return url;
    }

    tracing::warn!(
        "Could not resolve homeserver for {}; falling back to the configured URL",
        state.homeserver
    );
    state.homeserver.trim_end_matches('/').to_string()
}

/// What a native client (e.g. Element X iOS) provides to authenticate using Matrix.
#[derive(Debug, Deserialize, ToSchema)]
pub struct MatrixLoginRequest {
    /// The active Matrix access token from the user's Element X session.
    pub access_token: String,
    /// Optional homeserver URL (e.g. "https://matrix.org"). If omitted, defaults
    /// to the bot's configured homeserver.
    pub homeserver: Option<String>,
}

/// The session issued upon successful Matrix authentication.
#[derive(Debug, Serialize, ToSchema)]
pub struct MatrixLoginResponse {
    pub session_token: String,
    pub account_id: String,
    pub display_name: String,
    pub mxid: String,
}

#[derive(Deserialize)]
struct MatrixWhoamiResponse {
    user_id: String,
}

/// Authenticates a user using their active Matrix session from Element X iOS.
///
/// Verifies the token with the homeserver's `/_matrix/client/v3/account/whoami`,
/// resolves or provisions their local account row, and issues a standard session token.
#[utoipa::path(
    post,
    path = "/api/v1/auth/matrix/login",
    request_body = MatrixLoginRequest,
    responses(
        (status = 200, description = "Authenticated via Matrix", body = MatrixLoginResponse),
        (status = 400, description = "Invalid request payload", body = ApiErrorResponse),
        (status = 401, description = "Invalid or expired Matrix credentials", body = ApiErrorResponse)
    )
)]
pub async fn login_with_matrix(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(payload): Json<MatrixLoginRequest>,
) -> Result<axum::response::Response, ApiError> {
    let token = payload.access_token.trim();
    if token.is_empty() {
        return Err(ApiError::bad_request("access_token cannot be empty"));
    }
    if token.len() > 500 {
        return Err(ApiError::bad_request("access_token exceeds 500 characters"));
    }

    let configured_hs = resolved_homeserver(&state).await;
    let target_hs = match payload.homeserver {
        Some(hs) => {
            let trimmed = hs.trim().trim_end_matches('/');
            if trimmed.is_empty() || trimmed == configured_hs.trim_end_matches('/') {
                configured_hs
            } else {
                return Err(ApiError::bad_request(
                    "Custom homeserver is not supported; login must use the configured homeserver",
                ));
            }
        }
        None => configured_hs,
    };

    let whoami = verify_homeserver_whoami(&state.http_client, &target_hs, token).await?;
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok());

    issue_matrix_session(&state, &whoami.user_id, user_agent).await
}

async fn verify_homeserver_whoami(
    client: &reqwest::Client,
    homeserver: &str,
    token: &str,
) -> Result<MatrixWhoamiResponse, ApiError> {
    let whoami_url = format!("{}/_matrix/client/v3/account/whoami", homeserver);
    let res = client
        .get(&whoami_url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| {
            tracing::error!("Failed connecting to homeserver {}: {:?}", whoami_url, err);
            ApiError::internal("Could not verify Matrix credentials with homeserver")
        })?;

    if !res.status().is_success() {
        return Err(ApiError::unauthorized(
            "Invalid or expired Matrix credentials",
        ));
    }

    res.json().await.map_err(|err| {
        tracing::error!("Invalid whoami JSON response from homeserver: {:?}", err);
        ApiError::internal("Invalid response from Matrix homeserver")
    })
}

/// Issues a session for a verified Matrix user.
pub async fn issue_matrix_session(
    state: &ApiState,
    mxid: &str,
    user_agent: Option<&str>,
) -> Result<axum::response::Response, ApiError> {
    let accounts = &state.services.accounts;
    let internal = |err: &dyn std::fmt::Debug| {
        tracing::error!("Failed resolving account for {}: {:?}", mxid, err);
        ApiError::internal("Failed to resolve user account")
    };
    let identity = PlatformIdentity::matrix(mxid).map_err(|err| internal(&err))?;
    let account_id = match state
        .services
        .identity
        .resolve(&identity)
        .await
        .map_err(|err| internal(&err))?
    {
        Resolution::Account(account_id) => account_id,
        // Matrix provisions every sender, so this cannot happen.
        Resolution::Unlinked => return Err(internal(&"matrix identity left unlinked")),
    };

    let display_name = accounts
        .find(&account_id)
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?
        .map(|a| a.display_name)
        .unwrap_or_else(|| mxid.to_string());

    let session_token = state
        .services
        .accounts
        .create_unverified_session(&account_id, user_agent.or(Some("ElementX-iOS")), None, None)
        .await
        .map_err(|err| ApiError::internal(format!("Could not open a session: {err}")))?;

    let body = Json(MatrixLoginResponse {
        session_token: session_token.clone(),
        account_id: account_id.to_string(),
        display_name,
        mxid: mxid.to_string(),
    });

    Ok(with_session_cookie(
        body,
        &session_token,
        state.cookie_secure,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        // Tests never touch the network: the resolved homeserver is pre-filled
        // (so no `.well-known` discovery request is made) with a closed
        // loopback port, and the client gives up fast if one is ever sent.
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(500))
            .build()
            .expect("test http client");
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "https://mbm.one".to_string(),
            client,
        );
        *state.resolved_homeserver.write().await = Some("http://127.0.0.1:9".to_string());
        state
    }

    #[tokio::test]
    async fn test_issue_matrix_session_provisions_and_sets_cookie() {
        let state = test_state().await;

        let res = issue_matrix_session(&state, "@alice:example.org", Some("ElementX/1.0"))
            .await
            .unwrap();

        let set_cookie = res
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("must set cookie");
        let token =
            crate::domain::accounts::cookie_value(set_cookie, crate::domain::accounts::COOKIE_NAME)
                .expect("cookie must contain token");

        let account_id = state
            .services
            .identity
            .find_matrix_for_test("@alice:example.org")
            .await;
        assert!(account_id.is_some());

        let principal = state
            .services
            .accounts
            .resolve_credential(&crate::domain::accounts::Credential::BearerSession(token))
            .await
            .unwrap()
            .expect("session must resolve");
        let user = crate::interface::http::auth::AuthUser::from(principal);
        assert!(
            !user.may_perform_sensitive_action(chrono::Utc::now()),
            "Matrix access-token session must not satisfy passkey step-up"
        );
        assert_eq!(
            user.last_verified_at,
            chrono::DateTime::<chrono::Utc>::UNIX_EPOCH,
            "Matrix access-token session must be stamped as never verified"
        );
        assert!(user.never_presented_passkey());
    }

    #[tokio::test]
    async fn test_matrix_login_empty_token() {
        let state = test_state().await;

        let res = login_with_matrix(
            State(state),
            HeaderMap::new(),
            Json(MatrixLoginRequest {
                access_token: "   ".to_string(),
                homeserver: None,
            }),
        )
        .await;

        let Err(err) = res else {
            panic!("expected bad request error");
        };
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_matrix_login_invalid_homeserver_scheme() {
        let state = test_state().await;

        let res = login_with_matrix(
            State(state),
            HeaderMap::new(),
            Json(MatrixLoginRequest {
                access_token: "some_token".to_string(),
                homeserver: Some("ftp://invalid.example".to_string()),
            }),
        )
        .await;

        let Err(err) = res else {
            panic!("expected bad request error");
        };
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_matrix_login_token_too_long() {
        let state = test_state().await;

        let long_token = "a".repeat(501);
        let res = login_with_matrix(
            State(state),
            HeaderMap::new(),
            Json(MatrixLoginRequest {
                access_token: long_token,
                homeserver: None,
            }),
        )
        .await;

        let Err(err) = res else {
            panic!("expected bad request error");
        };
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_matrix_login_rejects_arbitrary_homeserver() {
        let state = test_state().await;

        let res = login_with_matrix(
            State(state),
            HeaderMap::new(),
            Json(MatrixLoginRequest {
                access_token: "valid_looking_token".to_string(),
                homeserver: Some("https://evil-attacker-homeserver.com".to_string()),
            }),
        )
        .await;

        let Err(err) = res else {
            panic!("expected bad request error for arbitrary homeserver");
        };
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("Custom homeserver is not supported"));
    }
}
