use axum::extract::FromRef;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use utoipa::ToSchema;

use crate::db::Database;
use crate::error::ApiError;

/// Ceiling on a single outbound homeserver request, start to finish.
const HTTP_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling on establishing the TCP/TLS connection alone.
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

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
    /// Application services, built once in `app.rs`. Handlers call these
    /// instead of reaching repositories or the pool.
    pub services: crate::application::AppServices,
    pub broadcaster: crate::events::EventBroadcaster,
    pub registration_secret: Option<String>,
    pub registration_open: bool,
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
    /// In memory, not in the database: a ceremony lives for seconds and is worthless
    /// afterwards, so durability would buy nothing and cost a table to sweep.
    /// This mirrors how `matrix_identity_cache` above is held.
    pub webauthn_ceremonies: std::sync::Arc<tokio::sync::RwLock<HashMap<String, PendingCeremony>>>,
    /// Whether the session cookie is marked `Secure`. False only in development.
    pub cookie_secure: bool,
    /// Whether to believe `X-Forwarded-For` from a trusted private peer.
    ///
    /// Read from configuration rather than from the environment at the point of
    /// use, so it cannot disagree with anything else and `Config` can see it.
    pub trust_forwarded_for: bool,
    /// The origin the SPA is served from, compared against `Origin` on every
    /// state-changing request.
    pub webauthn_origin: String,
    /// Accounts with a merge awaiting confirmation, `into -> from`.
    ///
    /// Written when `!link` finds the MXID already attached to an unclaimed
    /// account holding data, and read when the web UI asks what it should be
    /// showing. In memory rather than a table because it is a handoff between
    /// two halves of one flow: a stale entry should evaporate on restart rather
    /// than linger as a standing authorisation to absorb an account.
    pub pending_merges: std::sync::Arc<tokio::sync::RwLock<HashMap<String, String>>>,
    /// When the Matrix sync loop last completed a sync, read by `/health/ready`.
    pub sync_health: crate::interface::channels::matrix::sync_health::SyncHealth,
    /// The API server's shutdown token. Long-lived responses (SSE) end on it,
    /// or graceful shutdown would wait on every open browser tab.
    pub shutdown: tokio_util::sync::CancellationToken,
    pub ocr_config: crate::modules::ocr::OcrConfig,
    pub ocr: crate::infrastructure::inference::ocr::OcrClient,
    pub voice_config: crate::modules::voice::VoiceConfig,
    pub stt: crate::infrastructure::inference::stt::SttClient,
    /// AI provider settings resolved once at boot. See `crate::config::AiEnvConfig`.
    pub ai: crate::config::AiEnvConfig,
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
    /// Test fixture. Production builds its state with [`Self::from_config`].
    #[cfg(test)]
    pub fn new(
        db: Database,
        broadcaster: crate::events::EventBroadcaster,
        registration_secret: Option<String>,
        homeserver: String,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            services: crate::application::AppServices::new(&db.pool),
            db,
            broadcaster,
            registration_secret,
            registration_open: true,
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
            trust_forwarded_for: false,
            webauthn_origin: "http://localhost:8080".to_string(),
            pending_merges: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            sync_health: crate::interface::channels::matrix::sync_health::SyncHealth::default(),
            shutdown: tokio_util::sync::CancellationToken::new(),
            ocr_config: crate::modules::ocr::OcrConfig::default(),
            ocr: crate::infrastructure::inference::ocr::OcrClient::new(None),
            voice_config: crate::modules::voice::VoiceConfig::default(),
            stt: crate::infrastructure::inference::stt::SttClient::new(
                None,
                None,
                "sensevoice-small".to_string(),
            ),
            ai: crate::config::AiEnvConfig::default(),
        }
    }

    /// Builds the state a running server uses, from configuration.
    ///
    /// The only constructor production has. [`Self::new`] is a test fixture and
    /// carries development defaults — an open registration policy and a
    /// `localhost` Relying Party — that a caller then had to remember to
    /// override. Deriving every one of those from `Config` here means a missed
    /// builder call is no longer a way to ship them.
    pub fn from_config(
        config: &crate::config::Config,
        db: Database,
        services: crate::application::AppServices,
        broadcaster: crate::events::EventBroadcaster,
        handles: crate::interface::channels::matrix::handles::BotHandles,
        shutdown: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Self> {
        // Bound outbound calls so a stalled homeserver cannot pin a request task
        // indefinitely -- but generously. Reaching a homeserver through a VPN or
        // proxy can take several seconds just to establish the TCP connection
        // (measured at ~6s against one such setup), and a connect timeout tighter
        // than that silently breaks profile and media lookups.
        let http_client = reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(HTTP_TOTAL_TIMEOUT)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());

        // The Relying Party is a boot-time decision: an RP ID that does not
        // match the origin the browser loads mints credentials no browser will
        // return, so a bad pair must stop the process rather than surface as
        // failed sign-ins.
        let webauthn = build_webauthn(&config.webauthn_rp_id, &config.webauthn_origin)?;

        Ok(Self {
            db,
            services,
            broadcaster,
            registration_secret: config.registration_secret.clone(),
            registration_open: config.registration_open,
            homeserver: config.homeserver.clone(),
            http_client,
            bot_access_token: handles.access_token,
            bot_client: handles.client,
            resolved_homeserver: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            webauthn: std::sync::Arc::new(webauthn),
            webauthn_ceremonies: std::sync::Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            cookie_secure: config.cookie_secure,
            trust_forwarded_for: config.trust_forwarded_for,
            webauthn_origin: config.webauthn_origin.trim_end_matches('/').to_string(),
            pending_merges: handles.pending_merges,
            sync_health: handles.sync_health,
            shutdown,
            ocr_config: crate::modules::ocr::OcrConfig {
                enabled: config.ocr_enabled,
                base_url: config.ocr_base_url.clone(),
            },
            ocr: crate::infrastructure::inference::ocr::OcrClient::new(config.ocr_base_url.clone()),
            voice_config: crate::modules::voice::VoiceConfig {
                enabled: config.voice_enabled,
                language: config.voice_language.clone(),
            },
            stt: crate::infrastructure::inference::stt::SttClient::new(
                config.stt_base_url.clone(),
                config.stt_api_key.clone(),
                config.stt_model.clone(),
            ),
            ai: config.ai.clone(),
        })
    }

    /// Configures whether open registration without an invite secret is permitted.
    #[cfg(test)]
    pub fn with_registration_open(mut self, open: bool) -> Self {
        self.registration_open = open;
        self
    }

    /// Configures OCR and voice transcription module settings.
    #[cfg(test)]
    pub fn with_ocr_and_voice(
        mut self,
        ocr_config: crate::modules::ocr::OcrConfig,
        voice_config: crate::modules::voice::VoiceConfig,
    ) -> Self {
        self.ocr_config = ocr_config;
        self.voice_config = voice_config;
        self
    }

    /// Points this state at the configured Relying Party.
    ///
    /// Separate from `new` so the many test call sites keep working against the
    /// localhost default, exactly as `with_matrix_identity` does.
    #[cfg(test)]
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

    /// Checks whether an Origin header is permitted for state-changing requests.
    pub fn is_allowed_origin(&self, origin: &str) -> bool {
        let trimmed = origin.trim_end_matches('/');
        if trimmed == self.webauthn_origin.trim_end_matches('/') {
            return true;
        }
        if (self.webauthn_origin.contains("localhost")
            || self.webauthn_origin.contains("127.0.0.1"))
            && (trimmed == "http://localhost:5173"
                || trimmed == "http://localhost:8080"
                || trimmed == "http://127.0.0.1:5173"
                || trimmed == "http://127.0.0.1:8080")
        {
            return true;
        }
        false
    }

    /// Files a ceremony away and returns the id the browser quotes back.
    ///
    /// Expired entries are swept here rather than by a timer: ceremonies only
    /// accumulate when someone starts one, so the work belongs on that path.
    pub async fn stash_ceremony(&self, ceremony: Ceremony) -> Result<String, ApiError> {
        let id = crate::utils::random::random_hex(16);

        let now = Instant::now();
        let mut ceremonies = self.webauthn_ceremonies.write().await;
        ceremonies.retain(|_, pending| pending.expires_at > now);
        ceremonies.insert(
            id.clone(),
            PendingCeremony {
                ceremony,
                expires_at: now + CEREMONY_TTL,
            },
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

    let mut builder = webauthn_rs::WebauthnBuilder::new(rp_id, &parsed)?.rp_name("Matrix Bot");

    if rp_id == "localhost" || rp_id == "127.0.0.1" {
        builder = builder.allow_any_port(true);
        if let Ok(alt_8080) = url::Url::parse("http://localhost:8080") {
            builder = builder.append_allowed_origin(&alt_8080);
        }
        if let Ok(alt_5173) = url::Url::parse("http://localhost:5173") {
            builder = builder.append_allowed_origin(&alt_5173);
        }
    }

    Ok(builder.build()?)
}

impl FromRef<ApiState> for Database {
    fn from_ref(state: &ApiState) -> Self {
        state.db.clone()
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// A `Config` a production deployment would actually run with, spelled out
    /// field by field. Deliberately not `..Default::default()`: a field added
    /// to `Config` later must be decided here, not silently inherited.
    fn production_config() -> Config {
        Config {
            homeserver: "https://matrix.example.com".to_string(),
            username: "bot".to_string(),
            password: "secretpassword".to_string(),
            recovery_key: None,
            verification_allowlist: Vec::new(),
            database_url: "postgres://test@localhost/test".to_string(),
            api_host: "0.0.0.0".to_string(),
            api_port: 8080,
            registration_secret: None,
            registration_open: false,
            db_max_connections: 10,
            cors_allowed_origins: Vec::new(),
            webauthn_rp_id: "bot.example.com".to_string(),
            webauthn_origin: "https://bot.example.com".to_string(),
            cookie_secure: true,
            web_url: "https://bot.example.com".to_string(),
            ocr_enabled: false,
            ocr_base_url: None,
            voice_enabled: false,
            voice_language: "auto".to_string(),
            stt_base_url: None,
            stt_api_key: None,
            stt_model: "sensevoice-small".to_string(),
            tts_base_url: None,
            tts_api_key: None,
            tts_model: "kokoro".to_string(),
            tts_voice: "0".to_string(),
            tts_dialect: crate::infrastructure::inference::tts::TtsDialect::OpenAi,
            trust_forwarded_for: true,
            ai: crate::config::AiEnvConfig::default(),
        }
    }

    /// `new` is a test fixture whose defaults are deliberately permissive:
    /// registration open, a `localhost` Relying Party, an insecure cookie.
    /// Production used to start from those and undo them with builder calls, so
    /// a forgotten call was a way to ship them. This asserts `from_config`
    /// derives every one of them instead.
    #[tokio::test]
    async fn test_from_config_takes_every_security_default_from_config() {
        let pool = crate::db::pool::test_pool().await;
        let config = production_config();

        let state = ApiState::from_config(
            &config,
            crate::db::Database { pool: pool.clone() },
            crate::application::AppServices::new(&pool),
            crate::events::EventBroadcaster::default(),
            crate::interface::channels::matrix::handles::BotHandles::default(),
            tokio_util::sync::CancellationToken::new(),
        )
        .expect("a valid Relying Party must build");

        assert!(!state.registration_open, "registration must stay closed");
        assert!(
            state.cookie_secure,
            "the session cookie must be marked Secure"
        );
        assert!(state.trust_forwarded_for);
        assert_eq!(state.webauthn_origin, "https://bot.example.com");
        assert!(
            !state.is_allowed_origin("http://localhost:5173"),
            "a production Relying Party must not accept the dev origins"
        );

        // The Relying Party object itself, not just the origin string. Its RP
        // ID is baked into every credential, and `build_webauthn` widens the
        // allowed origins when the RP is `localhost`. Asserting the string
        // field alone let a hard-coded localhost Relying Party through.
        let origins: Vec<String> = state
            .webauthn
            .get_allowed_origins()
            .iter()
            .map(|url| url.to_string())
            .collect();
        assert!(
            origins.iter().all(|origin| !origin.contains("localhost")),
            "the Relying Party must come from config, not the localhost default: {origins:?}"
        );
    }

    /// A malformed origin must stop the process rather than surface later as
    /// every passkey login failing.
    #[tokio::test]
    async fn test_from_config_refuses_a_malformed_origin() {
        let pool = crate::db::pool::test_pool().await;
        let config = Config {
            webauthn_origin: "not-a-url".to_string(),
            ..production_config()
        };

        assert!(ApiState::from_config(
            &config,
            crate::db::Database { pool: pool.clone() },
            crate::application::AppServices::new(&pool),
            crate::events::EventBroadcaster::default(),
            crate::interface::channels::matrix::handles::BotHandles::default(),
            tokio_util::sync::CancellationToken::new(),
        )
        .is_err());
    }
}
