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

        let api_host = env::var("API_HOST")
            .unwrap_or_else(|_| "0.0.0.0".to_string());

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
        if self.jwt_secret.len() < 16 {
            tracing::warn!("JWT_SECRET is shorter than 16 characters — consider using a stronger secret in production");
        }
        Ok(())
    }
}
