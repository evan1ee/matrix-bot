use axum::extract::FromRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

use crate::db::Database;
use crate::error::ApiError;

/// An authenticated caller, resolved from the session cookie.
///
/// There is one identity space now. Callers used to arrive as either a Matrix
/// user or a local account, and `owner_id` was *derived* from whichever it was
/// — which is precisely what made one human two owners. The extractor resolves
/// a session to an account before any of this is read, so nothing here is
/// derived from anything.
#[derive(Debug, Clone)]
pub struct AuthUser {
    /// The `accounts.id` this caller acts as. The only ownership key there is.
    pub account_id: String,
    /// The human-facing name persisted in `sender_name`. Display only — it is
    /// never used to decide who owns a row.
    pub display_name: String,
    /// The session this request arrived on, so it can be revoked or stamped as
    /// freshly verified without looking it up again.
    pub session_id_hash: String,
    /// When the caller last proved possession of a passkey. The step-up clock.
    pub last_verified_at: chrono::DateTime<chrono::Utc>,
    /// Set when this session came out of account recovery and is still inside
    /// its restricted window.
    pub restricted_until: Option<chrono::DateTime<chrono::Utc>>,
}

impl AuthUser {
    /// The single canonical owner string persisted in `owner_id`.
    ///
    /// Every ownership check compares against this and nothing else.
    pub fn owner_id(&self) -> String {
        self.account_id.clone()
    }

    /// Whether a passkey assertion happened recently enough for a sensitive
    /// action, and this is not a restricted recovery session.
    ///
    /// Both conditions, not either: a recovery session is deliberately unable
    /// to reach a sensitive action however recently it authenticated.
    pub fn may_perform_sensitive_action(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        let recent = now.signed_duration_since(self.last_verified_at)
            < chrono::Duration::minutes(crate::accounts::session::STEP_UP_WINDOW_MINUTES);
        let restricted = self.restricted_until.is_some_and(|until| now < until);

        recent && !restricted
    }

    /// Test-only constructor. Production builds this from a session row.
    #[cfg(test)]
    pub fn for_account(account_id: impl Into<String>) -> Self {
        let account_id = account_id.into();
        Self {
            display_name: account_id.trim_start_matches("acct_").to_string(),
            account_id,
            session_id_hash: "test-session".to_string(),
            last_verified_at: chrono::Utc::now(),
            restricted_until: None,
        }
    }

    /// Test-only: a caller whose display name is set independently of the
    /// account, so tests can prove a shared name never implies shared data.
    #[cfg(test)]
    pub fn named(display_name: impl Into<String>, account_id: impl Into<String>) -> Self {
        Self {
            display_name: display_name.into(),
            ..Self::for_account(account_id)
        }
    }
}

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub broadcaster: crate::events::EventBroadcaster,
    pub registration_secret: Option<String>,
    pub homeserver: String,
    pub http_client: reqwest::Client,
    /// The **bot's** own Matrix access token, shared with the bot bootstrap.
    ///
    /// Written only by the bot's login path; request handlers read it to make
    /// bot-credentialed calls (`/rooms`, the media proxy). A request handler must
    /// never write a caller's token here — doing so silently repoints every
    /// bot-credentialed endpoint at whoever logged in most recently.
    pub bot_access_token: std::sync::Arc<tokio::sync::RwLock<Option<String>>>,
    /// The bot's Matrix SDK client, under the same ownership rule as above.
    pub bot_client: std::sync::Arc<tokio::sync::RwLock<Option<matrix_sdk::Client>>>,
    /// Homeserver base URL after `.well-known` discovery.
    ///
    /// Memoized because resolving it built a fresh `matrix_sdk::Client` — and so
    /// re-ran discovery — on every request carrying a Matrix token.
    pub resolved_homeserver: std::sync::Arc<tokio::sync::RwLock<Option<String>>>,
    /// The WebAuthn Relying Party. Its RP ID is baked into every credential.
    pub webauthn: std::sync::Arc<webauthn_rs::Webauthn>,
    /// WebAuthn ceremonies waiting for the browser to come back.
    ///
    /// In memory, not in SQLite: a ceremony lives for seconds and is worthless
    /// afterwards, so durability would buy nothing and cost a table to sweep.
    /// This mirrors how `matrix_identity_cache` above is held.
    pub webauthn_ceremonies:
        std::sync::Arc<tokio::sync::RwLock<HashMap<String, PendingCeremony>>>,
    /// Whether the session cookie is marked `Secure`. False only in development.
    pub cookie_secure: bool,
    /// The origin the SPA is served from, compared against `Origin` on every
    /// state-changing request.
    pub webauthn_origin: String,
}

/// A WebAuthn ceremony in flight.
pub enum Ceremony {
    /// Registering the first passkey for an account that does not exist yet.
    ///
    /// The account id is minted at `start` because it *is* the WebAuthn user
    /// handle and has to be inside the challenge, but the row is not written
    /// until `finish` — so an abandoned registration leaves nothing behind.
    Registration {
        account_id: String,
        display_name: String,
        state: webauthn_rs::prelude::PasskeyRegistration,
    },
    /// A usernameless login. Nobody is identified yet; the credential itself
    /// carries the handle that says which account is signing in.
    Authentication {
        state: webauthn_rs::prelude::DiscoverableAuthentication,
    },
}

pub struct PendingCeremony {
    pub ceremony: Ceremony,
    pub expires_at: Instant,
}

/// How long a browser has to complete a ceremony it started.
///
/// Long enough to find a security key in a drawer, short enough that an
/// abandoned challenge is not sitting in memory for an afternoon.
pub const CEREMONY_TTL: Duration = Duration::from_secs(300);

impl ApiState {
    pub fn new(
        db: Database,
        broadcaster: crate::events::EventBroadcaster,
        registration_secret: Option<String>,
        homeserver: String,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            db,
            broadcaster,
            registration_secret,
            homeserver,
            http_client,
            bot_access_token: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            bot_client: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            resolved_homeserver: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            webauthn: std::sync::Arc::new(
                build_webauthn("localhost", "http://localhost:8080")
                    .expect("the localhost development Relying Party is always valid"),
            ),
            webauthn_ceremonies: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            cookie_secure: false,
            webauthn_origin: "http://localhost:8080".to_string(),
        }
    }

    /// Points this state at the configured Relying Party.
    ///
    /// Separate from `new` so the many test call sites keep working against the
    /// localhost default, exactly as `with_matrix_identity` does.
    pub fn with_webauthn(
        mut self,
        rp_id: &str,
        origin: &str,
        cookie_secure: bool,
    ) -> anyhow::Result<Self> {
        self.webauthn = std::sync::Arc::new(build_webauthn(rp_id, origin)?);
        self.cookie_secure = cookie_secure;
        self.webauthn_origin = origin.trim_end_matches('/').to_string();
        Ok(self)
    }

    /// The origin a request's `Origin` header must match.
    pub fn webauthn_origin_str(&self) -> &str {
        &self.webauthn_origin
    }

    /// Files a ceremony away and returns the id the browser quotes back.
    ///
    /// Expired entries are swept here rather than by a timer: ceremonies only
    /// accumulate when someone starts one, so the work belongs on that path.
    pub async fn stash_ceremony(&self, ceremony: Ceremony) -> Result<String, ApiError> {
        let id = sqlx::query_scalar::<_, String>("SELECT lower(hex(randomblob(16)))")
            .fetch_one(&self.db.pool)
            .await
            .map_err(|err| ApiError::internal(format!("could not start a ceremony: {err}")))?;

        let now = Instant::now();
        let mut ceremonies = self.webauthn_ceremonies.write().await;
        ceremonies.retain(|_, pending| pending.expires_at > now);
        ceremonies.insert(
            id.clone(),
            PendingCeremony { ceremony, expires_at: now + CEREMONY_TTL },
        );

        Ok(id)
    }

    /// Takes a ceremony back out, consuming it.
    ///
    /// Removal is unconditional: a challenge is single-use, so a failed finish
    /// must not leave it replayable.
    pub async fn take_ceremony(&self, id: &str) -> Option<Ceremony> {
        let pending = self.webauthn_ceremonies.write().await.remove(id)?;
        (pending.expires_at > Instant::now()).then_some(pending.ceremony)
    }

}

/// Builds the Relying Party.
///
/// Subdomain matching is deliberately left off. Allowing it would let a
/// credential minted for the bot's host be presented from any sibling
/// subdomain, and the origin binding it would loosen is precisely what makes a
/// passkey unphishable.
pub fn build_webauthn(rp_id: &str, origin: &str) -> anyhow::Result<webauthn_rs::Webauthn> {
    let parsed = url::Url::parse(origin)
        .map_err(|err| anyhow::anyhow!("WEBAUTHN_ORIGIN '{origin}' is not a URL: {err}"))?;

    Ok(webauthn_rs::WebauthnBuilder::new(rp_id, &parsed)?
        .rp_name("Matrix Bot")
        .build()?)
}

impl FromRef<ApiState> for Database {
    fn from_ref(state: &ApiState) -> Self {
        state.db.clone()
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SseTicketResponse {
    /// Present this as `?ticket=` when opening `/api/v1/events`.
    pub ticket: String,
    /// Seconds until the ticket expires.
    pub expires_in: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserProfileResponse {
    /// The signed-in account. The frontend keys its caches on this.
    pub account_id: String,
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    /// Matrix identities linked to this account, if any. An empty list is the
    /// normal state for someone who has only ever used the web UI.
    pub linked_mxids: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct MediaProxyQuery {
    pub server: String,
    pub media_id: String,
}
