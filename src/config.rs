use anyhow::{Context, Result};
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub homeserver: String,
    pub username: String,
    pub password: String,
    pub recovery_key: Option<String>,
    pub database_url: String,
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

        Ok(Self {
            homeserver,
            username,
            password,
            recovery_key,
            database_url,
        })
    }
}
