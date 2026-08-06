use anyhow::{Context, Result};
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub homeserver: String,
    pub username: String,
    pub password: String,
    pub recovery_key: Option<String>,
    pub database_url: String,
    pub api_host: String,
    pub api_port: u16,
    pub jwt_secret: String,
    pub registration_secret: Option<String>,
    pub db_max_connections: u32,
    /// Origins permitted to call the API cross-origin.
    ///
    /// Empty by default: the SPA is served by this same server, and the Vite dev
    /// server proxies `/api`, so neither needs cross-origin access. Set
    /// `CORS_ALLOWED_ORIGINS` only for a genuinely separate front-end host.
    pub cors_allowed_origins: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        let homeserver = env::var("MATRIX_HOMESERVER")
            .unwrap_or_else(|_| "https://matrix.org".to_string());

        let username = env::var("MATRIX_USERNAME")
            .context("MATRIX_USERNAME environment variable is required")?;

        let password = env::var("MATRIX_PASSWORD")
            .context("MATRIX_PASSWORD environment variable is required")?;

        let recovery_key = env::var("MATRIX_RECOVERY_KEY").ok();

        let database_url = env::var("DATABASE_URL")
            .unwrap_or_else(|_| "sqlite://data/bot_data.db?mode=rwc".to_string());

        let api_host = env::var("API_HOST").unwrap_or_else(|_| "0.0.0.0".to_string());

        let api_port: u16 = env::var("API_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(8080);

        let jwt_secret = env::var("JWT_SECRET")
            .unwrap_or_else(|_| "matrix_bot_secret_jwt_token_key_change_in_production".to_string());

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

        Ok(Self {
            homeserver,
            username,
            password,
            recovery_key,
            database_url,
            api_host,
            api_port,
            jwt_secret,
            registration_secret,
            db_max_connections,
            cors_allowed_origins,
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

        let is_prod = env::var("ENVIRONMENT").map(|v| v.to_lowercase() == "production").unwrap_or(false)
            || cfg!(not(debug_assertions));

        if is_prod && self.jwt_secret == "matrix_bot_secret_jwt_token_key_change_in_production" {
            anyhow::bail!("Insecure default JWT_SECRET used in production environment. Set a custom JWT_SECRET!");
        }

        if self.jwt_secret.len() < 16 {
            tracing::warn!("JWT_SECRET is shorter than 16 characters — consider using a stronger secret in production");
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
            database_url: "sqlite::memory:".to_string(),
            api_host: "127.0.0.1".to_string(),
            api_port: 8080,
            jwt_secret: "super_secret_jwt_key_12345".to_string(),
            registration_secret: None,
            db_max_connections: 5,
            cors_allowed_origins: Vec::new(),
        };
        assert!(valid_cfg.validate().is_ok());

        let invalid_cfg = Config {
            homeserver: "".to_string(),
            ..valid_cfg.clone()
        };
        assert!(invalid_cfg.validate().is_err());
    }
}
