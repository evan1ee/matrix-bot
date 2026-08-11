//! Passkey registration and usernameless login.
//!
//! Four endpoints, two ceremonies. Each starts by handing the browser a
//! challenge and filing the server's half away under a ceremony id; each
//! finishes by quoting that id back with the authenticator's response.
//!
//! Login is *discoverable*: the browser is asked for any credential belonging
//! to this Relying Party, and the credential itself carries the account handle.
//! There is no username field anywhere in the app, which also means there is no
//! user enumeration to defend against — an unknown account and a wrong
//! credential are the same answer.

use axum::response::IntoResponse;
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use webauthn_rs::prelude::{
    DiscoverableKey, PublicKeyCredential, RegisterPublicKeyCredential,
};

use crate::accounts::{
    handle_to_account_id, session::COOKIE_NAME, AccountRepository, CredentialRepository,
    SessionRepository,
};
use crate::api::auth::types::{ApiState, Ceremony};
use crate::error::ApiError;

/// What a browser needs to begin registering a passkey.
#[derive(Debug, Serialize, ToSchema)]
pub struct CeremonyStart {
    /// Quoted back on finish, so the server can find its half of the ceremony.
    pub ceremony_id: String,
    /// The raw WebAuthn options, passed to `navigator.credentials`.
    #[schema(value_type = Object)]
    pub options: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterStartRequest {
    /// The name to show on this account. Display only.
    pub display_name: String,
    /// The invite secret, when the deployment requires one.
    pub invite: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterFinishRequest {
    pub ceremony_id: String,
    /// The authenticator's response, verbatim.
    #[schema(value_type = Object)]
    pub credential: RegisterPublicKeyCredential,
    /// What to call this passkey in the account's own list.
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct LoginFinishRequest {
    pub ceremony_id: String,
    #[schema(value_type = Object)]
    pub credential: PublicKeyCredential,
}

/// Who the caller is now signed in as.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub account_id: String,
    pub display_name: String,
}

/// Rejects a registration that did not present the configured invite secret.
///
/// Deliberately checked before anything is minted, so a wrong secret cannot
/// leave a ceremony or an account id behind.
fn check_invite(state: &ApiState, provided: Option<&str>) -> Result<(), ApiError> {
    let Some(required) = state.registration_secret.as_deref() else {
        return Ok(());
    };

    if provided.unwrap_or("") == required {
        Ok(())
    } else {
        Err(ApiError::unauthorized("Invalid registration secret"))
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/passkey/register/start",
    request_body = RegisterStartRequest,
    responses(
        (status = 200, description = "Challenge to pass to navigator.credentials.create", body = CeremonyStart),
        (status = 401, description = "Invalid invite secret", body = ApiErrorResponse)
    )
)]
pub async fn register_start(
    State(state): State<ApiState>,
    Json(payload): Json<RegisterStartRequest>,
) -> Result<Json<CeremonyStart>, ApiError> {
    check_invite(&state, payload.invite.as_deref())?;

    let display_name = payload.display_name.trim();
    if display_name.is_empty() || display_name.len() > 64 {
        return Err(ApiError::bad_request("Display name must be 1-64 characters"));
    }

    // The account id is minted now because it *is* the WebAuthn user handle and
    // has to travel inside the challenge — but the row is not written until the
    // ceremony finishes, so an abandoned registration leaves nothing behind.
    let account_id = sqlx::query_scalar::<_, String>(
        "SELECT 'acct_' || lower(hex(randomblob(16)))",
    )
    .fetch_one(&state.db.pool)
    .await
    .map_err(|err| ApiError::internal(format!("could not mint an account id: {err}")))?;

    let handle = crate::accounts::account_id_to_handle(&account_id)
        .ok_or_else(|| ApiError::internal("minted an account id with no handle"))?;

    let (options, reg_state) = state
        .webauthn
        .start_passkey_registration(handle, display_name, display_name, None)
        .map_err(|err| {
            tracing::error!("Could not start passkey registration: {:?}", err);
            ApiError::internal("Could not start passkey registration")
        })?;

    let ceremony_id = state
        .stash_ceremony(Ceremony::Registration {
            account_id,
            display_name: display_name.to_string(),
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
    post,
    path = "/api/v1/auth/passkey/register/finish",
    request_body = RegisterFinishRequest,
    responses(
        (status = 200, description = "Account created and signed in", body = SessionResponse),
        (status = 400, description = "Ceremony expired or credential rejected", body = ApiErrorResponse)
    )
)]
pub async fn register_finish(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<RegisterFinishRequest>,
) -> Result<axum::response::Response, ApiError> {
    let Some(Ceremony::Registration { account_id, display_name, state: reg_state }) =
        state.take_ceremony(&payload.ceremony_id).await
    else {
        return Err(ApiError::bad_request(
            "That registration expired. Start again.",
        ));
    };

    let passkey = state
        .webauthn
        .finish_passkey_registration(&payload.credential, &reg_state)
        .map_err(|err| {
            tracing::warn!("Passkey registration rejected: {:?}", err);
            ApiError::bad_request("That passkey could not be verified")
        })?;

    // Only now does the account become real.
    AccountRepository::create_with_id(
        &state.db.pool,
        &account_id,
        &display_name,
        crate::accounts::STATUS_ACTIVE,
    )
    .await
    .map_err(|err| ApiError::internal(format!("could not create the account: {err}")))?;

    let label = payload
        .label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .unwrap_or("Passkey");

    CredentialRepository::add(&state.db.pool, &account_id, &passkey, label)
        .await
        .map_err(|err| ApiError::internal(format!("could not store the passkey: {err}")))?;

    issue_session(&state, &account_id, &display_name, &headers).await
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/passkey/login/start",
    responses(
        (status = 200, description = "Challenge to pass to navigator.credentials.get", body = CeremonyStart)
    )
)]
pub async fn login_start(
    State(state): State<ApiState>,
) -> Result<Json<CeremonyStart>, ApiError> {
    let (options, auth_state) = state
        .webauthn
        .start_discoverable_authentication()
        .map_err(|err| {
            tracing::error!("Could not start discoverable authentication: {:?}", err);
            ApiError::internal("Could not start sign-in")
        })?;

    let ceremony_id = state
        .stash_ceremony(Ceremony::Authentication { state: auth_state })
        .await?;

    Ok(Json(CeremonyStart {
        ceremony_id,
        options: serde_json::to_value(&options)
            .map_err(|err| ApiError::internal(err.to_string()))?,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/auth/passkey/login/finish",
    request_body = LoginFinishRequest,
    responses(
        (status = 200, description = "Signed in", body = SessionResponse),
        (status = 401, description = "Credential rejected", body = ApiErrorResponse)
    )
)]
pub async fn login_finish(
    State(state): State<ApiState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<LoginFinishRequest>,
) -> Result<axum::response::Response, ApiError> {
    let Some(Ceremony::Authentication { state: auth_state }) =
        state.take_ceremony(&payload.ceremony_id).await
    else {
        return Err(ApiError::bad_request("That sign-in expired. Try again."));
    };

    // The credential says which account it belongs to. Nothing was typed, and
    // nothing was guessed — the handle was written into the credential when it
    // was created.
    let (handle, _cred_id) = state
        .webauthn
        .identify_discoverable_authentication(&payload.credential)
        .map_err(|err| {
            tracing::warn!("Could not identify a discoverable credential: {:?}", err);
            ApiError::unauthorized("Sign-in failed")
        })?;

    let account_id = handle_to_account_id(&handle);

    let stored = CredentialRepository::list_for_account(&state.db.pool, &account_id)
        .await
        .map_err(|err| ApiError::internal(format!("could not read credentials: {err}")))?;

    let discoverable: Vec<DiscoverableKey> = stored
        .iter()
        .filter_map(|c| c.passkey())
        .map(|p| DiscoverableKey::from(&p))
        .collect();

    if discoverable.is_empty() {
        // Either the account has no passkeys or it does not exist. The caller
        // is told the same thing either way.
        return Err(ApiError::unauthorized("Sign-in failed"));
    }

    let result = state
        .webauthn
        .finish_discoverable_authentication(&payload.credential, auth_state, &discoverable)
        .map_err(|err| {
            tracing::warn!("Discoverable authentication rejected: {:?}", err);
            ApiError::unauthorized("Sign-in failed")
        })?;

    // The signature counter is the authenticator's clone-detection signal, so a
    // successful assertion that moved it has to be written back. Dropping this
    // would quietly disable that protection.
    if result.needs_update() {
        if let Some(row) = stored
            .iter()
            .find(|c| c.passkey().is_some_and(|p| p.cred_id() == result.cred_id()))
        {
            if let Some(mut passkey) = row.passkey() {
                passkey.update_credential(&result);
                if let Err(err) =
                    CredentialRepository::record_use(&state.db.pool, &row.id, &passkey).await
                {
                    tracing::error!("Could not persist the signature counter: {:?}", err);
                }
            }
        }
    }

    let account = AccountRepository::find(&state.db.pool, &account_id)
        .await
        .map_err(|err| ApiError::internal(format!("could not read the account: {err}")))?
        .ok_or_else(|| ApiError::unauthorized("Sign-in failed"))?;

    issue_session(&state, &account_id, &account.display_name, &headers).await
}

/// Opens a session and returns it as a `Set-Cookie` alongside the body.
///
/// `HttpOnly` keeps the value out of JavaScript's reach; `SameSite=Strict`
/// means a cross-site request never carries it, which is most of the CSRF
/// story. `Secure` follows the deployment — see `Config::cookie_secure` for why
/// it is off in development.
async fn issue_session(
    state: &ApiState,
    account_id: &str,
    display_name: &str,
    headers: &axum::http::HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    let user_agent = headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|v| v.to_str().ok());

    let token = SessionRepository::create(&state.db.pool, account_id, user_agent, None, None)
        .await
        .map_err(|err| ApiError::internal(format!("could not open a session: {err}")))?;

    let body = Json(SessionResponse {
        account_id: account_id.to_string(),
        display_name: display_name.to_string(),
    });

    Ok(with_session_cookie(body, &token, state.cookie_secure))
}

/// Attaches the session cookie to a response.
pub fn with_session_cookie(
    body: impl axum::response::IntoResponse,
    token: &str,
    secure: bool,
) -> axum::response::Response {
    let max_age = crate::accounts::session::ABSOLUTE_TIMEOUT_DAYS * 24 * 60 * 60;
    let cookie = format!(
        "{COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    );

    let mut response = body.into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().append(axum::http::header::SET_COOKIE, value);
    }
    response
}

/// Clears the session cookie.
pub fn clearing_session_cookie(secure: bool) -> axum::response::Response {
    let cookie = format!(
        "{COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    );

    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&cookie) {
        response.headers_mut().append(axum::http::header::SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    async fn state_with_secret(secret: Option<&str>) -> ApiState {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            secret.map(str::to_string),
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        )
    }

    /// Registration is invite-gated, and the check happens before anything is
    /// minted — a wrong secret must not leave a ceremony behind.
    #[tokio::test]
    async fn test_registration_requires_the_invite_secret() {
        let state = state_with_secret(Some("letmein")).await;

        let err = register_start(
            State(state.clone()),
            Json(RegisterStartRequest {
                display_name: "evan".to_string(),
                invite: Some("wrong".to_string()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);

        assert!(
            state.webauthn_ceremonies.read().await.is_empty(),
            "a rejected registration must not leave a ceremony in memory"
        );

        // The right secret gets a challenge.
        let ok = register_start(
            State(state.clone()),
            Json(RegisterStartRequest {
                display_name: "evan".to_string(),
                invite: Some("letmein".to_string()),
            }),
        )
        .await
        .unwrap();
        assert!(!ok.ceremony_id.is_empty());
    }

    /// With no secret configured, anyone may register.
    #[tokio::test]
    async fn test_registration_is_open_when_no_secret_is_set() {
        let state = state_with_secret(None).await;
        let started = register_start(
            State(state),
            Json(RegisterStartRequest {
                display_name: "evan".to_string(),
                invite: None,
            }),
        )
        .await
        .unwrap();
        assert!(!started.ceremony_id.is_empty());
    }

    /// Starting registration must not write an account: an abandoned ceremony
    /// would otherwise litter the table with rows nobody can sign in to.
    #[tokio::test]
    async fn test_starting_registration_creates_no_account() {
        let state = state_with_secret(None).await;

        let _ = register_start(
            State(state.clone()),
            Json(RegisterStartRequest { display_name: "evan".to_string(), invite: None }),
        )
        .await
        .unwrap();

        let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
            .fetch_one(&state.db.pool)
            .await
            .unwrap();
        assert_eq!(accounts, 0, "the account is written only when the ceremony finishes");
    }

    #[tokio::test]
    async fn test_display_name_is_bounded() {
        let state = state_with_secret(None).await;

        for bad in ["", "   ", &"x".repeat(65)] {
            let err = register_start(
                State(state.clone()),
                Json(RegisterStartRequest { display_name: bad.to_string(), invite: None }),
            )
            .await
            .unwrap_err();
            assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
        }
    }

    /// A challenge is single-use. Quoting the same ceremony id twice must fail
    /// the second time even if the first attempt errored.
    #[tokio::test]
    async fn test_a_ceremony_id_cannot_be_replayed() {
        let state = state_with_secret(None).await;
        let started = register_start(
            State(state.clone()),
            Json(RegisterStartRequest { display_name: "evan".to_string(), invite: None }),
        )
        .await
        .unwrap();

        assert!(state.take_ceremony(&started.ceremony_id).await.is_some());
        assert!(
            state.take_ceremony(&started.ceremony_id).await.is_none(),
            "a consumed ceremony must not be usable again"
        );
    }

    /// Login asks for a credential without being told who is signing in, so an
    /// unknown account and a wrong credential are indistinguishable.
    #[tokio::test]
    async fn test_login_start_needs_no_identity() {
        let state = state_with_secret(None).await;
        let started = login_start(State(state.clone())).await.unwrap();

        assert!(!started.ceremony_id.is_empty());
        assert!(
            matches!(
                state.webauthn_ceremonies.read().await.get(&started.ceremony_id),
                Some(pending) if matches!(pending.ceremony, Ceremony::Authentication { .. })
            ),
            "login must file an authentication ceremony, not a registration"
        );
    }

    /// An expired ceremony is refused rather than honoured late.
    #[tokio::test]
    async fn test_an_expired_ceremony_is_refused() {
        let state = state_with_secret(None).await;
        let started = login_start(State(state.clone())).await.unwrap();

        // Age the ceremony past its TTL.
        {
            let mut ceremonies = state.webauthn_ceremonies.write().await;
            if let Some(pending) = ceremonies.get_mut(&started.ceremony_id) {
                pending.expires_at = std::time::Instant::now() - std::time::Duration::from_secs(1);
            }
        }

        assert!(state.take_ceremony(&started.ceremony_id).await.is_none());
    }

    /// The cookie must carry the flags that make it useless to JavaScript and
    /// to cross-site requests. This is the CSRF and XSS story in one string.
    #[test]
    fn test_session_cookie_flags() {
        let response = with_session_cookie(axum::http::StatusCode::OK, "tok123", true);
        let cookie = response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        assert!(cookie.contains("mb_session=tok123"));
        assert!(cookie.contains("HttpOnly"), "must be unreadable by JavaScript");
        assert!(cookie.contains("SameSite=Strict"), "must not ride cross-site requests");
        assert!(cookie.contains("Secure"), "production must be HTTPS-only");
        assert!(cookie.contains("Path=/"));

        // Development drops only `Secure`, because Safari refuses it on
        // http://localhost.
        let dev = with_session_cookie(axum::http::StatusCode::OK, "tok123", false);
        let dev_cookie = dev
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(!dev_cookie.contains("Secure"));
        assert!(dev_cookie.contains("HttpOnly"));
    }

    #[test]
    fn test_clearing_cookie_expires_it_immediately() {
        let response = clearing_session_cookie(true);
        let cookie = response
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        assert!(cookie.contains("Max-Age=0"), "sign-out must expire the cookie");
        assert!(cookie.contains("mb_session="));
    }
}
