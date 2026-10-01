use anyhow::{Context, Result};
use std::collections::HashMap;
use std::env;

use crate::domain::channels::Channel;

const DEFAULT_STT_MODEL: &str = "sensevoice-small";
const DEFAULT_TTS_MODEL: &str = "kokoro";
const DEFAULT_TTS_VOICE: &str = "0";

/// Whether this process is running as a production deployment.
///
/// Read in two places — the `Secure` cookie flag and the boot-time refusal to
/// run on development WebAuthn settings — which must never disagree, so it is
/// one function rather than two copies of the same expression.
fn is_production() -> bool {
    match env::var("ENVIRONMENT").map(|v| v.to_lowercase()).as_deref() {
        Ok("development") | Ok("local") | Ok("test") => false,
        Ok("production") => true,
        _ => cfg!(not(debug_assertions)),
    }
}

/// Resolves the registration policy based on release/debug mode, invite secret, and open flags.
///
/// In release / production builds, open registration is disabled by default to protect
/// self-hosted instances against automated open-internet scanning.
pub fn resolve_registration_policy(
    is_prod: bool,
    invite_secret: Option<String>,
    registration_open_env: Option<String>,
    allow_open_flag: bool,
) -> (Option<String>, bool) {
    let secret = invite_secret
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let open = if allow_open_flag {
        true
    } else if let Some(env_val) = registration_open_env {
        let v = env_val.trim();
        v.eq_ignore_ascii_case("true")
            || v == "1"
            || v.eq_ignore_ascii_case("yes")
            || v.eq_ignore_ascii_case("on")
    } else if is_prod {
        false
    } else {
        secret.is_none()
    };

    (secret, open)
}

/// The application database is PostgreSQL only; refuse anything else at boot
/// rather than failing on the first query.
pub fn validate_database_url(url: &str) -> anyhow::Result<()> {
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        Ok(())
    } else {
        anyhow::bail!(
            "DATABASE_URL must be a PostgreSQL URL (postgres://user:pass@host:5432/db); got {:?}",
            url.split('@').next_back().unwrap_or(url)
        )
    }
}

/// The database URL, required and validated.
///
/// MCP mode does not build a full `Config` — it needs no Matrix credentials —
/// but it must not carry its own copy of this validation either, or the two
/// silently diverge.
pub fn database_url_from_env() -> Result<String> {
    let database_url = env::var("DATABASE_URL")
        .map_err(|_| anyhow::anyhow!("DATABASE_URL is required (a postgres:// URL)"))?;
    validate_database_url(&database_url)?;
    Ok(database_url)
}

/// AI provider settings that come from the environment.
///
/// The endpoint, key and model each have exactly one name. Resolved once at
/// boot so every caller sees the same answer.
#[derive(Debug, Clone, Default)]
pub struct AiEnvConfig {
    /// `AI_ENDPOINT`: the OpenAI-compatible base URL, including `/v1`.
    pub base_url: Option<String>,
    /// `AI_API_KEY`, ignoring a blank value.
    pub api_key: Option<String>,
    /// `AI_MODEL`.
    pub model: Option<String>,
    /// First of AI_PROXY, HTTPS_PROXY, https_proxy, ALL_PROXY, all_proxy,
    /// HTTP_PROXY, http_proxy.
    pub proxy: Option<String>,
    /// Whether an AI base URL may point at a private or loopback address.
    ///
    /// Off by default: a user-supplied base URL is a server-side request this
    /// process will make, so an unguarded one reaches anything the host can.
    pub allow_private_base_url: bool,
}

impl AiEnvConfig {
    pub fn from_env() -> Self {
        let base_url = env::var("AI_ENDPOINT").ok();

        let api_key = env::var("AI_API_KEY").ok().filter(|s| !s.trim().is_empty());

        let model = env::var("AI_MODEL").ok();

        let proxy = env::var("AI_PROXY")
            .or_else(|_| env::var("HTTPS_PROXY"))
            .or_else(|_| env::var("https_proxy"))
            .or_else(|_| env::var("ALL_PROXY"))
            .or_else(|_| env::var("all_proxy"))
            .or_else(|_| env::var("HTTP_PROXY"))
            .or_else(|_| env::var("http_proxy"))
            .ok();

        let allow_private_base_url = env::var("AI_ALLOW_PRIVATE_BASE_URL")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        Self {
            base_url,
            api_key,
            model,
            proxy,
            allow_private_base_url,
        }
    }
}

/// Job queue workers per instance when `JOB_WORKERS` is unset.
pub const DEFAULT_JOB_WORKERS: usize = 4;
/// Upper bound on `JOB_WORKERS`; each running job holds a pool connection.
pub const MAX_JOB_WORKERS: usize = 32;

fn parse_job_workers(raw: Option<&str>) -> Result<usize> {
    match raw {
        None => Ok(DEFAULT_JOB_WORKERS),
        Some(v) => v
            .trim()
            .parse::<usize>()
            .with_context(|| format!("JOB_WORKERS must be a whole number (got '{v}')")),
    }
}

/// The signup allowlist, one environment variable per channel, each a
/// comma-separated list. Entries are trimmed, empty ones dropped, email
/// addresses lowercased (the canonical form identities are stored in); a
/// channel with no entries is left out of the map.
fn parse_link_signup_allowlist(
    get: impl Fn(&str) -> Option<String>,
) -> HashMap<Channel, Vec<String>> {
    [
        (Channel::Discord, "DISCORD_ALLOWED_USERS"),
        (Channel::Telegram, "TELEGRAM_ALLOWED_USERS"),
        (Channel::Email, "EMAIL_ALLOWED_SENDERS"),
    ]
    .into_iter()
    .filter_map(|(channel, name)| {
        let entries: Vec<String> = get(name)?
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(|entry| match channel {
                Channel::Email => entry.to_lowercase(),
                _ => entry.to_owned(),
            })
            .collect();
        (!entries.is_empty()).then_some((channel, entries))
    })
    .collect()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub homeserver: String,
    pub username: String,
    pub password: String,
    pub recovery_key: Option<String>,
    /// Users permitted to complete an interactive verification with the bot.
    /// The bot's own account is always permitted; see `bot/verification.rs`.
    pub verification_allowlist: Vec<String>,
    pub database_url: String,
    pub api_host: String,
    pub api_port: u16,
    pub registration_secret: Option<String>,
    pub registration_open: bool,
    pub db_max_connections: u32,
    /// Concurrent job-queue workers in this instance (`JOB_WORKERS`, 1..=32, default 4).
    pub job_workers: usize,
    /// Host part of each worker id (`<host>:<pid>:<uuid>`), from `HOSTNAME`/`COMPUTERNAME`; diagnostic only.
    pub job_worker_host: String,
    /// Origins permitted to call the API cross-origin.
    ///
    /// Empty by default: the SPA is served by this same server, and the Vite dev
    /// server proxies `/api`, so neither needs cross-origin access. Set
    /// `CORS_ALLOWED_ORIGINS` only for a genuinely separate front-end host.
    pub cors_allowed_origins: Vec<String>,
    /// The WebAuthn Relying Party ID — a bare hostname, never a URL or a port.
    ///
    /// Baked into every credential at creation: change it and every passkey
    /// ever registered stops working. Ports are not part of it, which is why
    /// one value of `localhost` covers the SPA on `:5173` and the API on
    /// `:8080` with no special casing.
    pub webauthn_rp_id: String,
    /// The full origin the browser loads the page from, port included.
    ///
    /// Unlike the RP ID this *does* carry the scheme and port, and WebAuthn
    /// checks it exactly.
    pub webauthn_origin: String,
    /// Whether to mark the session cookie `Secure`.
    ///
    /// Derived from `ENVIRONMENT` rather than configured separately, because
    /// the two must never disagree. Chrome and Firefox accept `Secure` cookies
    /// on `http://localhost`, but Safari has historically not — so development
    /// leaves it off and production always sets it.
    pub cookie_secure: bool,
    /// Where to send someone whose command needs the web app.
    ///
    /// Defaults to the WebAuthn origin, which is by definition where the SPA
    /// is served from — so it is right without being configured twice.
    pub web_url: String,
    /// Identities that may register a new account from a link token (spec D4),
    /// per channel: `DISCORD_ALLOWED_USERS`, `TELEGRAM_ALLOWED_USERS`,
    /// `EMAIL_ALLOWED_SENDERS` (comma-separated). Absent channels and unset
    /// variables mean nobody may; the values are never logged.
    pub link_signup_allowlist: HashMap<Channel, Vec<String>>,
    /// Whether OCR image text recognition is enabled.
    pub ocr_enabled: bool,
    /// Base URL of the inference server's OCR endpoint.
    ///
    /// Absent means OCR is unavailable in this deployment: the feature
    /// degrades with a message, and nothing else is affected.
    pub ocr_base_url: Option<String>,
    /// Whether speech transcription is enabled.
    pub voice_enabled: bool,
    /// Default speech recognition language (e.g. "auto", "zh", "en", "ja", "ko", "yue").
    pub voice_language: String,
    /// Base URL of an OpenAI-compatible speech-to-text API, including its
    /// version prefix (`http://inference:8090/v1`, `https://api.openai.com/v1`).
    /// Absent means voice transcription is unavailable in this deployment.
    pub stt_base_url: Option<String>,
    /// Bearer token for a hosted STT provider. The bundled server needs none.
    pub stt_api_key: Option<String>,
    /// Model name sent with every transcription request. The bundled server
    /// has one model and ignores it.
    pub stt_model: String,
    /// Base URL of a text-to-speech API, including its version prefix
    /// (`http://inference:8090/v1`, `https://api.openai.com/v1`,
    /// `https://api.elevenlabs.io/v1`). Absent means `!say` is unavailable in
    /// this deployment.
    pub tts_base_url: Option<String>,
    /// API key for a hosted provider: a bearer token under the OpenAI
    /// dialect, `xi-api-key` under ElevenLabs. The bundled server needs none.
    pub tts_api_key: Option<String>,
    /// Model name sent with each request. The bundled server ignores it.
    pub tts_model: String,
    /// The house voice, for anyone who never chose one: a Kokoro speaker id
    /// for the bundled server, a name such as `alloy` for OpenAI, a voice id
    /// for ElevenLabs.
    pub tts_voice: String,
    /// How requests are shaped for the provider at `tts_base_url`.
    pub tts_dialect: crate::infrastructure::inference::tts::TtsDialect,
    /// Whether to believe `X-Forwarded-For`/`X-Real-IP` from a trusted peer.
    ///
    /// Forwarded headers are believed only when this is on *and* the connecting
    /// peer is itself a private/loopback address (a reverse proxy on the same
    /// host or LAN) — so it defaults off and is safe to leave off behind a
    /// setup that does not put a trusted proxy in front of the bot.
    pub trust_forwarded_for: bool,
    pub ai: AiEnvConfig,
}

impl Config {
    /// Builds the process configuration from already-loaded environment
    /// variables.
    ///
    /// This never loads a `.env` file itself — `main.rs` is the only place
    /// that does, and only from the current directory (`load_dotenv_from`),
    /// never dotenvy's own upward parent-directory search. An earlier version
    /// called dotenvy's parent-searching `dotenv()` function here too, which
    /// could silently pick up a different checkout's `.env` (e.g. the main
    /// checkout's production credentials, when run from a worktree under it).
    pub fn from_env() -> Result<Self> {
        let homeserver =
            env::var("MATRIX_HOMESERVER").unwrap_or_else(|_| "https://matrix.org".to_string());

        let username = env::var("MATRIX_USERNAME")
            .context("MATRIX_USERNAME environment variable is required")?;

        let password = env::var("MATRIX_PASSWORD")
            .context("MATRIX_PASSWORD environment variable is required")?;

        let recovery_key = env::var("MATRIX_RECOVERY_KEY").ok();

        let verification_allowlist = env::var("MATRIX_VERIFICATION_ALLOWLIST")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let database_url = database_url_from_env()?;

        let api_host = env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let api_port: u16 = env::var("API_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);

        let invite_secret = env::var("INVITE_SECRET")
            .or_else(|_| env::var("API_REGISTRATION_SECRET"))
            .ok();

        let registration_open_env = env::var("REGISTRATION_OPEN")
            .or_else(|_| env::var("ALLOW_OPEN_REGISTRATION"))
            .ok();

        let allow_open_flag = env::args().any(|a| a == "--allow-open-registration");

        let (registration_secret, registration_open) = resolve_registration_policy(
            is_production(),
            invite_secret,
            registration_open_env,
            allow_open_flag,
        );

        let db_max_connections: u32 = env::var("DATABASE_MAX_CONNECTIONS")
            .ok()
            .and_then(|c| c.parse().ok())
            .unwrap_or(10);

        let job_workers = parse_job_workers(env::var("JOB_WORKERS").ok().as_deref())?;
        let job_worker_host = env::var("HOSTNAME")
            .or_else(|_| env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown-host".to_string());

        let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let webauthn_rp_id = env::var("WEBAUTHN_RP_ID").unwrap_or_else(|_| "localhost".to_string());

        let webauthn_origin =
            env::var("WEBAUTHN_ORIGIN").unwrap_or_else(|_| "http://localhost:8080".to_string());

        let cookie_secure = is_production();

        let web_url = env::var("WEB_URL").unwrap_or_else(|_| webauthn_origin.clone());

        let link_signup_allowlist = parse_link_signup_allowlist(|name| env::var(name).ok());

        let ocr_enabled = env::var("OCR_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        let ocr_base_url = env::var("OCR_BASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());

        let voice_enabled = env::var("VOICE_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        let voice_language = env::var("VOICE_LANGUAGE").unwrap_or_else(|_| "auto".to_string());

        let stt_base_url = env::var("STT_BASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let stt_api_key = env::var("STT_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let stt_model = env::var("STT_MODEL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_STT_MODEL.to_string());

        let tts_base_url = env::var("TTS_BASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let tts_api_key = env::var("TTS_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let tts_model = env::var("TTS_MODEL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_TTS_MODEL.to_string());
        let tts_voice = env::var("TTS_VOICE")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_TTS_VOICE.to_string());
        let tts_dialect = crate::infrastructure::inference::tts::TtsDialect::parse(
            &env::var("TTS_DIALECT").unwrap_or_default(),
        )
        .map_err(|e| anyhow::anyhow!(e))?;

        let trust_forwarded_for = env::var("TRUST_FORWARDED_FOR")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        let ai = AiEnvConfig::from_env();

        Ok(Self {
            homeserver,
            username,
            password,
            recovery_key,
            verification_allowlist,
            database_url,
            api_host,
            api_port,
            registration_secret,
            registration_open,
            db_max_connections,
            job_workers,
            job_worker_host,
            cors_allowed_origins,
            webauthn_rp_id,
            webauthn_origin,
            cookie_secure,
            web_url,
            link_signup_allowlist,
            ocr_enabled,
            ocr_base_url,
            voice_enabled,
            voice_language,
            stt_base_url,
            stt_api_key,
            stt_model,
            tts_base_url,
            tts_api_key,
            tts_model,
            tts_voice,
            tts_dialect,
            trust_forwarded_for,
            ai,
        })
    }

    /// The text-to-speech client for this deployment.
    pub fn tts_client(&self) -> crate::infrastructure::inference::tts::TtsClient {
        crate::infrastructure::inference::tts::TtsClient::new(
            self.tts_base_url.clone(),
            self.tts_api_key.clone(),
            self.tts_model.clone(),
            self.tts_dialect,
        )
    }

    pub fn validate(&self) -> Result<()> {
        if self.homeserver.is_empty() {
            anyhow::bail!("MATRIX_HOMESERVER cannot be empty");
        }
        if self.username.is_empty() {
            anyhow::bail!("MATRIX_USERNAME cannot be empty");
        }
        if self.password.is_empty() {
            anyhow::bail!("MATRIX_PASSWORD cannot be empty");
        }

        if !(1..=MAX_JOB_WORKERS).contains(&self.job_workers) {
            anyhow::bail!(
                "JOB_WORKERS must be between 1 and {MAX_JOB_WORKERS} (got {})",
                self.job_workers
            );
        }

        let is_prod = is_production();

        // A production deployment still on the development Relying Party would
        // mint passkeys bound to `localhost`, which no real browser would ever
        // send back to it — every registration would appear to succeed and
        // every subsequent login would fail. Refuse to boot instead.
        if is_prod
            && (self.webauthn_rp_id == "localhost" || self.webauthn_origin.contains("localhost"))
        {
            anyhow::bail!(
                "WEBAUTHN_RP_ID/WEBAUTHN_ORIGIN are still the localhost defaults in production. \
                 Set them to the real host — they are baked into every passkey and cannot be \
                 changed later without invalidating all of them."
            );
        }

        if self.webauthn_rp_id.contains("://") || self.webauthn_rp_id.contains(':') {
            anyhow::bail!(
                "WEBAUTHN_RP_ID must be a bare hostname with no scheme or port (got '{}')",
                self.webauthn_rp_id
            );
        }

        if !self.webauthn_origin.contains("://") {
            anyhow::bail!(
                "WEBAUTHN_ORIGIN must be a full origin including the scheme (got '{}')",
                self.webauthn_origin
            );
        }

        Ok(())
    }
}

#[cfg(test)]
impl Config {
    /// A valid configuration that reaches no network, for tests that need a `Config`.
    pub(crate) fn for_test() -> Self {
        Self {
            homeserver: "https://matrix.org".to_string(),
            username: "bot".to_string(),
            password: "secretpassword".to_string(),
            recovery_key: None,
            verification_allowlist: Vec::new(),
            database_url: "postgres://test@localhost/test".to_string(),
            api_host: "127.0.0.1".to_string(),
            api_port: 8080,
            registration_secret: None,
            registration_open: true,
            db_max_connections: 5,
            job_workers: 4,
            job_worker_host: "test".into(),
            cors_allowed_origins: Vec::new(),
            webauthn_rp_id: "localhost".to_string(),
            webauthn_origin: "http://localhost:8080".to_string(),
            cookie_secure: false,
            web_url: "http://localhost:8080".to_string(),
            link_signup_allowlist: HashMap::new(),
            ocr_enabled: true,
            ocr_base_url: None,
            voice_enabled: true,
            voice_language: "auto".to_string(),
            stt_base_url: None,
            stt_api_key: None,
            stt_model: "sensevoice-small".to_string(),
            tts_base_url: None,
            tts_api_key: None,
            tts_model: "kokoro".to_string(),
            tts_voice: "0".to_string(),
            tts_dialect: crate::infrastructure::inference::tts::TtsDialect::OpenAi,
            trust_forwarded_for: false,
            ai: AiEnvConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_config() -> Config {
        Config::for_test()
    }

    #[test]
    fn test_job_workers_must_be_between_1_and_32() {
        let base = valid_config();
        for bad in [0, 33] {
            assert!(
                Config {
                    job_workers: bad,
                    ..base.clone()
                }
                .validate()
                .is_err(),
                "{bad}"
            );
        }
        for good in [1, 4, 32] {
            assert!(
                Config {
                    job_workers: good,
                    ..base.clone()
                }
                .validate()
                .is_ok(),
                "{good}"
            );
        }
    }

    #[test]
    fn the_link_signup_allowlist_reads_one_env_name_per_channel() {
        use crate::domain::channels::Channel;
        let env = |name: &str| match name {
            "DISCORD_ALLOWED_USERS" => Some(" 111 , 222,, ".to_string()),
            "TELEGRAM_ALLOWED_USERS" => Some("42".to_string()),
            "EMAIL_ALLOWED_SENDERS" => Some("Evan@Example.com, ,b@x.org".to_string()),
            _ => None,
        };
        let list = parse_link_signup_allowlist(env);
        assert_eq!(list[&Channel::Discord], vec!["111", "222"]);
        assert_eq!(list[&Channel::Telegram], vec!["42"]);
        assert_eq!(
            list[&Channel::Email],
            vec!["evan@example.com", "b@x.org"],
            "email entries are lowercased"
        );
        assert!(!list.contains_key(&Channel::Matrix));
    }

    #[test]
    fn the_link_signup_allowlist_is_empty_when_unset_or_blank() {
        assert!(parse_link_signup_allowlist(|_| None).is_empty());
        assert!(parse_link_signup_allowlist(|_| Some(" , ,".to_string())).is_empty());
    }

    #[test]
    fn test_parse_job_workers() {
        assert_eq!(parse_job_workers(None).unwrap(), DEFAULT_JOB_WORKERS);
        assert_eq!(parse_job_workers(Some("8")).unwrap(), 8);
        assert!(parse_job_workers(Some("four")).is_err());
    }

    #[test]
    fn test_config_validation() {
        let valid_cfg = valid_config();
        assert!(valid_cfg.validate().is_ok());

        let invalid_cfg = Config {
            homeserver: "".to_string(),
            ..valid_cfg.clone()
        };
        assert!(invalid_cfg.validate().is_err());
    }

    /// The RP ID is a hostname. A URL or a port here mints credentials that no
    /// browser will ever return, and the failure only shows up at login — long
    /// after registration appeared to succeed.
    #[test]
    fn test_rp_id_must_be_a_bare_hostname() {
        let base = Config {
            homeserver: "https://mbm.one".to_string(),
            username: "bot".to_string(),
            password: "secretpassword".to_string(),
            recovery_key: None,
            verification_allowlist: Vec::new(),
            database_url: "postgres://test@localhost/test".to_string(),
            api_host: "127.0.0.1".to_string(),
            api_port: 8080,
            registration_secret: None,
            registration_open: true,
            db_max_connections: 5,
            job_workers: 4,
            job_worker_host: "test".into(),
            cors_allowed_origins: Vec::new(),
            webauthn_rp_id: "localhost".to_string(),
            webauthn_origin: "http://localhost:8080".to_string(),
            cookie_secure: false,
            web_url: "http://localhost:8080".to_string(),
            link_signup_allowlist: HashMap::new(),
            ocr_enabled: true,
            ocr_base_url: None,
            voice_enabled: true,
            voice_language: "auto".to_string(),
            stt_base_url: None,
            stt_api_key: None,
            stt_model: "sensevoice-small".to_string(),
            tts_base_url: None,
            tts_api_key: None,
            tts_model: "kokoro".to_string(),
            tts_voice: "0".to_string(),
            tts_dialect: crate::infrastructure::inference::tts::TtsDialect::OpenAi,
            trust_forwarded_for: false,
            ai: AiEnvConfig::default(),
        };

        for bad in ["https://mbm.one", "mbm.one:8080", "localhost:5173"] {
            let cfg = Config {
                webauthn_rp_id: bad.to_string(),
                ..base.clone()
            };
            assert!(
                cfg.validate().is_err(),
                "'{bad}' must be rejected as an RP ID"
            );
        }

        for good in ["localhost", "mbm.one", "bot.mbm.one"] {
            let cfg = Config {
                webauthn_rp_id: good.to_string(),
                ..base.clone()
            };
            assert!(cfg.validate().is_ok(), "'{good}' is a valid RP ID");
        }

        // The origin is the other half of the pair and does carry a scheme.
        let cfg = Config {
            webauthn_origin: "mbm.one".to_string(),
            ..base.clone()
        };
        assert!(
            cfg.validate().is_err(),
            "an origin without a scheme must be rejected"
        );
    }

    #[test]
    fn test_resolve_registration_policy_matrix() {
        // 1. Release / production mode defaults to closed without secrets or flags
        let (secret, open) = resolve_registration_policy(true, None, None, false);
        assert_eq!(secret, None);
        assert!(!open, "release build must default to registration closed");

        // 2. Release / production mode with invite secret enables invite-only
        let (secret, open) =
            resolve_registration_policy(true, Some("my_secret".to_string()), None, false);
        assert_eq!(secret.as_deref(), Some("my_secret"));
        assert!(
            !open,
            "invite secret means invite-only, not open registration"
        );

        // 3. Release / production mode with CLI flag --allow-open-registration
        let (secret, open) = resolve_registration_policy(true, None, None, true);
        assert_eq!(secret, None);
        assert!(
            open,
            "CLI flag --allow-open-registration opens registration"
        );

        // 4. Release / production mode with REGISTRATION_OPEN=true env var
        let (secret, open) =
            resolve_registration_policy(true, None, Some("true".to_string()), false);
        assert_eq!(secret, None);
        assert!(open, "REGISTRATION_OPEN=true opens registration in release");

        // 5. Release / production mode with REGISTRATION_OPEN=false env var
        let (secret, open) =
            resolve_registration_policy(true, None, Some("false".to_string()), false);
        assert_eq!(secret, None);
        assert!(!open, "REGISTRATION_OPEN=false keeps registration closed");

        // 6. Debug / local mode defaults to open when no secret is configured
        let (secret, open) = resolve_registration_policy(false, None, None, false);
        assert_eq!(secret, None);
        assert!(open, "debug/local mode defaults to open registration");

        // 7. Debug / local mode with explicit secret is invite-only
        let (secret, open) =
            resolve_registration_policy(false, Some("dev_secret".to_string()), None, false);
        assert_eq!(secret.as_deref(), Some("dev_secret"));
        assert!(!open, "debug mode with secret is invite-only");

        // 8. Debug / local mode with REGISTRATION_OPEN=false explicitly closes it
        let (secret, open) =
            resolve_registration_policy(false, None, Some("false".to_string()), false);
        assert_eq!(secret, None);
        assert!(
            !open,
            "debug mode with REGISTRATION_OPEN=false closes registration"
        );
    }

    /// Configuration is read in one place. Everything else is handed values.
    ///
    /// The bot half used to re-read `OCR_BASE_URL`, `VOICE_*` and the
    /// verification allowlist at the point of use, so it could disagree with
    /// the copy the API server was given and `validate` could not check what it
    /// did not own. The files below still read the environment directly and are
    /// listed rather than ignored: `search.rs` sits behind the AI tool executor,
    /// whose signature chain reaches MCP stdio mode where no `Config` exists,
    /// and is tracked separately; the three test entries only read in
    /// `#[ignore]`d live tests, which this scan cannot distinguish from
    /// production code. Shrink this list; do not grow it.
    #[test]
    fn test_only_config_reads_the_environment() {
        const ALLOWED: &[&str] = &[
            "config.rs",
            "search.rs",
            // Read only inside #[ignore]d live tests.
            "self_verify.rs",
            // PG_TEST_URL, in the #[ignore]d live PostgreSQL connection test.
            "pool.rs",
            // MIGRATE_TEST_DATABASE_URL, in the #[ignore]d production rehearsal test.
            "catalog.rs",
        ];

        fn walk(dir: &std::path::Path, offenders: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("src/ must be readable") {
                let path = entry.expect("a readable directory entry").path();
                if path.is_dir() {
                    walk(&path, offenders);
                    continue;
                }
                if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                    continue;
                }
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                if ALLOWED.contains(&name.as_str()) {
                    continue;
                }
                let source = std::fs::read_to_string(&path).expect("a readable source file");
                for (number, line) in source.lines().enumerate() {
                    if line.trim_start().starts_with("//") {
                        continue;
                    }
                    if line.contains(&["env", "var"].join("::")) {
                        offenders.push(format!("{}:{}", path.display(), number + 1));
                    }
                }
            }
        }

        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        walk(&src, &mut offenders);

        assert!(
            offenders.is_empty(),
            "only config.rs may read the environment; found: {offenders:#?}"
        );
    }

    /// `Config::from_env` must never load a `.env` file itself: `main.rs` is
    /// the sole loader, and only from the current directory. Flipping the
    /// process's current directory to prove `Config::from_env` ignores a
    /// parent `.env` would be unsound here — the test suite runs in parallel
    /// and a global CWD mutation would race every other test — so this
    /// asserts the stronger, race-free property directly from the source:
    /// `from_env` performs no `.env` loading at all by construction, which
    /// makes "does it search parent directories" moot.
    #[test]
    fn test_config_from_env_never_loads_dotenv() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/config.rs");
        let source = std::fs::read_to_string(&path).expect("config.rs must be readable");
        // Built at runtime, not as one string literal: the literal would itself
        // match a repo-wide `grep` for "the parent-searching dotenvy call",
        // which this project uses as its own drift check for this fix.
        let parent_searching_call = format!("{}::{}()", "dotenvy", "dotenv");
        assert!(
            !source.contains(&parent_searching_call),
            "Config::from_env (or anything else in config.rs) must not call \
             dotenvy's parent-searching dotenv function -- it walks up parent \
             directories looking for a .env file; only main.rs may load one, \
             and only from the current directory via load_dotenv_from"
        );
    }

    #[test]
    fn test_database_url_must_be_postgres() {
        assert!(validate_database_url("postgres://bot:bot@localhost/matrix_bot").is_ok());
        assert!(validate_database_url("postgresql://bot@db/matrix_bot").is_ok());

        let err = validate_database_url("sqlite://data/bot_data.db?mode=rwc").unwrap_err();
        assert!(
            err.to_string().contains("DATABASE_URL"),
            "error must name the variable: {err}"
        );
        assert!(validate_database_url("").is_err());
    }
}
