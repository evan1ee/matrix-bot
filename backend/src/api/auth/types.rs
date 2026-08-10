use axum::extract::FromRef;
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

use crate::db::Database;

/// Hashes a bearer token into a cache key, so no live credential is stored.
fn token_cache_key(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}

/// Which identity space a caller authenticated in.
///
/// This is recorded at authentication time rather than inferred later from the
/// shape of the subject string: a local account name and a Matrix user ID are
/// different namespaces, and guessing between them by looking for a leading `@`
/// is what previously let a local account impersonate a Matrix user.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    /// A locally registered `web_users` account.
    #[default]
    Local,
    /// A Matrix user, proven either by homeserver login or by access token.
    Matrix,
}

/// Local account names must never be mistakable for a Matrix user ID, because
/// `@` and `:` are exactly what keeps the two identity spaces apart in
/// [`AuthUser::owner_id`].
pub fn is_valid_local_username(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

/// What a token is allowed to do.
///
/// `EventSource` cannot set request headers, so the SSE stream has to take its
/// credential from the URL — where it lands in browser history, proxy logs and
/// `Referer`. Rather than put the 7-day API token there, callers exchange it for
/// a short-lived stream ticket that opens nothing else.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenScope {
    /// Full access to the REST API. Must be presented in the `Authorization` header.
    #[default]
    Api,
    /// Opens the SSE stream and nothing else.
    SseStream,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
    /// Required. A token issued before this field existed is **rejected**, not
    /// defaulted: defaulting it silently reinterpreted a Matrix subject as a
    /// local account, so the holder kept working but as a different principal —
    /// their rows went to `@web-@user:server:api` and their real data vanished
    /// from view. A 401 sends them back through login instead.
    pub kind: AuthKind,
    /// Defaults to `Api`, matching tokens issued before scoping existed.
    #[serde(default)]
    pub scope: TokenScope,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct AuthPayload {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct RegisterPayload {
    pub username: String,
    pub password: String,
    pub secret: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    pub token: String,
    pub token_type: String,
    pub username: String,
}

#[derive(Debug, Clone)]
pub struct AuthUser {
    /// The MXID for [`AuthKind::Matrix`], or the account name for [`AuthKind::Local`].
    pub subject: String,
    pub kind: AuthKind,
}

impl AuthUser {
    pub fn matrix(mxid: impl Into<String>) -> Self {
        Self { subject: mxid.into(), kind: AuthKind::Matrix }
    }

    /// Test-only: the production path builds local identities from signed JWT
    /// claims in the extractor, never from a bare username.
    #[cfg(test)]
    pub fn local(username: impl Into<String>) -> Self {
        Self { subject: username.into(), kind: AuthKind::Local }
    }

    /// The single canonical owner string persisted in `sender_mxid`.
    ///
    /// Every ownership check compares against this and nothing else. Local
    /// accounts are namespaced into `@web-*:api`, which registration validation
    /// guarantees a real MXID can never collide with.
    pub fn owner_id(&self) -> String {
        match self.kind {
            AuthKind::Matrix => self.subject.clone(),
            AuthKind::Local => format!("@web-{}:api", self.subject),
        }
    }

    /// The human-facing name persisted in `sender_name`. Display only — it is
    /// never used to decide who owns a row.
    pub fn display_name(&self) -> String {
        match self.kind {
            AuthKind::Matrix => self
                .subject
                .trim_start_matches('@')
                .split(':')
                .next()
                .unwrap_or(&self.subject)
                .to_string(),
            AuthKind::Local => self.subject.clone(),
        }
    }
}

pub struct JwtKeys {
    pub encoding: EncodingKey,
    pub decoding: DecodingKey,
}

impl JwtKeys {
    pub fn new(secret: &[u8]) -> Self {
        Self {
            encoding: EncodingKey::from_secret(secret),
            decoding: DecodingKey::from_secret(secret),
        }
    }
}

/// The homeserver interactions the auth layer depends on.
///
/// Behind a trait purely so those paths are testable: verifying that a Matrix
/// token yields `AuthKind::Matrix`, or that logging in leaves the bot's
/// credentials alone, otherwise requires a live homeserver — which is exactly
/// what kept this security boundary untested.
#[async_trait::async_trait]
pub trait MatrixIdentityProvider: Send + Sync {
    /// Resolves an access token to its Matrix user ID.
    async fn whoami(&self, state: &ApiState, token: &str) -> Option<String>;

    /// Authenticates with a password, yielding `(user_id, access_token)`.
    async fn login(&self, state: &ApiState, username: &str, password: &str)
        -> Option<(String, String)>;
}

/// Talks to the real homeserver.
pub struct HomeserverIdentityProvider;

#[async_trait::async_trait]
impl MatrixIdentityProvider for HomeserverIdentityProvider {
    async fn whoami(&self, state: &ApiState, token: &str) -> Option<String> {
        crate::api::auth::matrix::validate_matrix_access_token(state, token).await
    }

    async fn login(
        &self,
        state: &ApiState,
        username: &str,
        password: &str,
    ) -> Option<(String, String)> {
        crate::api::auth::matrix::authenticate_matrix_user(state, username, password).await
    }
}

#[derive(Clone)]
pub struct ApiState {
    pub db: Database,
    pub broadcaster: crate::events::EventBroadcaster,
    pub jwt_secret: String,
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
    /// How the auth layer reaches Matrix. Swapped in tests.
    pub matrix_identity: std::sync::Arc<dyn MatrixIdentityProvider>,
    /// Access token → MXID, so repeat calls skip the `/whoami` round trip.
    ///
    /// Keyed by SHA-256 of the token, never the token itself: this map outlives
    /// any single request, and a bearer token is a live credential that must not
    /// sit in a long-lived structure where a `Debug` print or a core dump would
    /// expose something replayable.
    pub matrix_identity_cache:
        std::sync::Arc<tokio::sync::RwLock<HashMap<[u8; 32], CachedIdentity>>>,
}

/// A resolved Matrix identity and the moment it stops being trusted.
pub struct CachedIdentity {
    user_id: String,
    expires_at: Instant,
}

/// How long a resolved Matrix identity is reused before re-asking the
/// homeserver.
///
/// This is the window in which a token revoked upstream (the user signed out of
/// their client) still reaches the API, so it is deliberately short. The cost it
/// removes is not one round trip but the burst: a screen that fires several
/// requests at once previously paid a `/whoami` for each, and homeservers rate
/// limit that endpoint.
const MATRIX_IDENTITY_TTL: Duration = Duration::from_secs(60);

/// Entries tolerated before expired ones are swept on insert.
///
/// The map is bounded by the number of distinct *valid* tokens — failures are
/// never cached, so an attacker replaying garbage cannot grow it.
const MATRIX_IDENTITY_CACHE_SWEEP_AT: usize = 512;

impl ApiState {
    pub fn new(
        db: Database,
        broadcaster: crate::events::EventBroadcaster,
        jwt_secret: String,
        registration_secret: Option<String>,
        homeserver: String,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            db,
            broadcaster,
            jwt_secret,
            registration_secret,
            homeserver,
            http_client,
            bot_access_token: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            bot_client: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            resolved_homeserver: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            matrix_identity: std::sync::Arc::new(HomeserverIdentityProvider),
            matrix_identity_cache: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
        }
    }

    /// Resolves a Matrix access token to its MXID, reusing a recent answer.
    ///
    /// Every Matrix-authenticated request used to spend a homeserver round trip
    /// here, so latency was doubled and a burst of parallel requests could trip
    /// the homeserver's rate limiter.
    ///
    /// Only successes are cached. A rejected token stays rejected on the fast
    /// path guard in `looks_like_access_token`, and caching a `None` would mean
    /// a token that becomes valid (or a homeserver blip) kept failing for the
    /// whole TTL.
    pub async fn resolve_matrix_identity(&self, token: &str) -> Option<String> {
        let key = token_cache_key(token);

        if let Some(hit) = self.matrix_identity_cache.read().await.get(&key) {
            if hit.expires_at > Instant::now() {
                return Some(hit.user_id.clone());
            }
        }

        // Concurrent first-time requests for the same token can each miss and
        // call out; that costs a few duplicate round trips on a cold token and
        // is left alone deliberately — collapsing them needs per-key locking,
        // which is more machinery than the saving justifies.
        let user_id = self.matrix_identity.whoami(self, token).await?;

        let mut cache = self.matrix_identity_cache.write().await;
        if cache.len() >= MATRIX_IDENTITY_CACHE_SWEEP_AT {
            let now = Instant::now();
            cache.retain(|_, entry| entry.expires_at > now);
        }
        cache.insert(
            key,
            CachedIdentity {
                user_id: user_id.clone(),
                expires_at: Instant::now() + MATRIX_IDENTITY_TTL,
            },
        );

        Some(user_id)
    }

    /// Replaces the Matrix identity provider, so tests can exercise the
    /// Matrix-credential paths without a homeserver.
    #[cfg(test)]
    pub fn with_matrix_identity(
        mut self,
        provider: std::sync::Arc<dyn MatrixIdentityProvider>,
    ) -> Self {
        self.matrix_identity = provider;
        self
    }
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
    pub username: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, utoipa::IntoParams)]
pub struct MediaProxyQuery {
    pub server: String,
    pub media_id: String,
}
