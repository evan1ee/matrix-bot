use anyhow::{Context, Result};
use std::env;

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

/// The database URL, with the default every entry point shares.
///
/// MCP mode does not build a full `Config` — it needs no Matrix credentials —
/// but it must not carry its own copy of this default either, or the two
/// silently diverge.
pub fn database_url_from_env() -> String {
    env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://data/bot_data.db?mode=rwc".to_string())
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
    /// Whether local PaddleOCR image text recognition is enabled.
    pub ocr_enabled: bool,
    /// Path to directory containing the PaddleOCR ONNX models.
    pub ocr_model_dir: std::path::PathBuf,
    /// Whether local sherpa-onnx SenseVoice speech transcription is enabled.
    pub voice_enabled: bool,
    /// Path to directory containing the SenseVoice ONNX model and tokens.
    pub voice_model_dir: std::path::PathBuf,
    /// Default speech recognition language (e.g. "auto", "zh", "en", "ja", "ko", "yue").
    pub voice_language: String,
    /// Whether to believe `X-Forwarded-For`/`X-Real-IP` from a trusted peer.
    ///
    /// Forwarded headers are believed only when this is on *and* the connecting
    /// peer is itself a private/loopback address (a reverse proxy on the same
    /// host or LAN) — so it defaults off and is safe to leave off behind a
    /// setup that does not put a trusted proxy in front of the bot.
    pub trust_forwarded_for: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

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

        let database_url = database_url_from_env();

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

        let ocr_enabled = env::var("OCR_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        let ocr_model_dir = env::var("OCR_MODEL_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("data/ocr_models"));

        let voice_enabled = env::var("VOICE_ENABLED")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(true);

        let voice_model_dir = env::var("VOICE_MODEL_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("data/voice_models"));

        let voice_language = env::var("VOICE_LANGUAGE").unwrap_or_else(|_| "auto".to_string());

        let trust_forwarded_for = env::var("TRUST_FORWARDED_FOR")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

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
            cors_allowed_origins,
            webauthn_rp_id,
            webauthn_origin,
            cookie_secure,
            web_url,
            ocr_enabled,
            ocr_model_dir,
            voice_enabled,
            voice_model_dir,
            voice_language,
            trust_forwarded_for,
        })
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
mod tests {
    use super::*;

    #[test]
    fn test_config_validation() {
        let valid_cfg = Config {
            homeserver: "https://matrix.org".to_string(),
            username: "bot".to_string(),
            password: "secretpassword".to_string(),
            recovery_key: None,
            verification_allowlist: Vec::new(),
            database_url: "sqlite::memory:".to_string(),
            api_host: "127.0.0.1".to_string(),
            api_port: 8080,
            registration_secret: None,
            registration_open: true,
            db_max_connections: 5,
            cors_allowed_origins: Vec::new(),
            webauthn_rp_id: "localhost".to_string(),
            webauthn_origin: "http://localhost:8080".to_string(),
            cookie_secure: false,
            web_url: "http://localhost:8080".to_string(),
            ocr_enabled: true,
            ocr_model_dir: std::path::PathBuf::from("data/ocr_models"),
            voice_enabled: true,
            voice_model_dir: std::path::PathBuf::from("data/voice_models"),
            voice_language: "auto".to_string(),
            trust_forwarded_for: false,
        };
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
            database_url: "sqlite::memory:".to_string(),
            api_host: "127.0.0.1".to_string(),
            api_port: 8080,
            registration_secret: None,
            registration_open: true,
            db_max_connections: 5,
            cors_allowed_origins: Vec::new(),
            webauthn_rp_id: "localhost".to_string(),
            webauthn_origin: "http://localhost:8080".to_string(),
            cookie_secure: false,
            web_url: "http://localhost:8080".to_string(),
            ocr_enabled: true,
            ocr_model_dir: std::path::PathBuf::from("data/ocr_models"),
            voice_enabled: true,
            voice_model_dir: std::path::PathBuf::from("data/voice_models"),
            voice_language: "auto".to_string(),
            trust_forwarded_for: false,
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
    /// The bot half used to re-read `OCR_MODEL_DIR`, `VOICE_*` and the
    /// verification allowlist at the point of use, so it could disagree with
    /// the copy the API server was given and `validate` could not check what it
    /// did not own. The files below still read the environment directly and are
    /// listed rather than ignored: the AI client resolves its endpoint through a
    /// chain of five fallbacks that has to move as one piece, and the two test
    /// files only read in `#[ignore]`d live tests. Shrink this list; do not grow
    /// it.
    #[test]
    fn test_only_config_reads_the_environment() {
        const ALLOWED: &[&str] = &[
            "config.rs",
            // Resolves AI_ENDPOINT/AI_API_BASE/OPENAI_* and proxy settings
            // through fallback chains that belong in Config; tracked separately.
            "client.rs",
            "models.rs",
            "search.rs",
            // Read only inside #[ignore]d live tests.
            "self_verify.rs",
            "backfill.rs",
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
}
