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

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/bot_data.db?mode=rwc".to_string());

        let api_host = env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let api_port: u16 = env::var("API_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);

        let registration_secret = env::var("API_REGISTRATION_SECRET").ok();

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

        let voice_language = env::var("VOICE_LANGUAGE")
            .unwrap_or_else(|_| "auto".to_string());

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
}
