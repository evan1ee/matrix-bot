use anyhow::{Context, Result};
use matrix_sdk::{
    ruma::{ServerName, UserId},
    Client,
};
use std::time::Duration;
use tracing::{info, warn};

use crate::config::Config;

pub struct BotClient;

impl BotClient {
    pub async fn build_and_login(config: &Config) -> Result<Client> {
        let homeserver_input = &config.homeserver;
        let clean_domain = homeserver_input
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        let client = if let Ok(server_name) = ServerName::parse(clean_domain) {
            info!("Attempting Server / Well-Known Discovery for: {}", server_name);
            match Client::builder()
                .server_name(&server_name)
                .sqlite_store("bot_store", None)
                .build()
                .await
            {
                Ok(c) => {
                    info!("Successfully resolved homeserver via Well-Known discovery for: {}", server_name);
                    c
                }
                Err(err) => {
                    info!("Well-known lookup failed ({}), falling back to direct URL {}", err, homeserver_input);
                    let fallback_url = if homeserver_input.starts_with("http://") || homeserver_input.starts_with("https://") {
                        homeserver_input.to_string()
                    } else {
                        format!("https://{}", clean_domain)
                    };

                    Client::builder()
                        .homeserver_url(&fallback_url)
                        .sqlite_store("bot_store", None)
                        .build()
                        .await
                        .context("Failed to build Matrix client with fallback URL")?
                }
            }
        } else {
            let fallback_url = if homeserver_input.starts_with("http://") || homeserver_input.starts_with("https://") {
                homeserver_input.to_string()
            } else {
                format!("https://{}", clean_domain)
            };
            info!("Using direct homeserver URL: {}", fallback_url);
            Client::builder()
                .homeserver_url(&fallback_url)
                .sqlite_store("bot_store", None)
                .build()
                .await
                .context("Failed to build Matrix client with direct URL")?
        };

        if client.matrix_auth().session().is_none() {
            info!("Logging in as user: {}", config.username);

            let user_id = if config.username.starts_with('@') {
                UserId::parse(&config.username)
                    .context("Failed to parse MATRIX_USERNAME as a valid Matrix User ID")?
            } else {
                let raw_id = format!("@{}:{}", config.username, clean_domain);
                UserId::parse(&raw_id).context("Failed to construct valid Matrix User ID")?
            };

            client
                .matrix_auth()
                .login_username(&user_id, &config.password)
                .device_id("RUST_HELLO_BOT_SESSION")
                .initial_device_display_name("Rust Matrix Bot")
                .send()
                .await
                .context("Failed to log in to Matrix server")?;

            info!("Successfully logged in as {}", client.user_id().unwrap());
        } else {
            info!("Restored existing session from store for user: {}", client.user_id().unwrap());
        }

        if let Some(key) = &config.recovery_key {
            let trimmed_key = key.trim().to_string();
            if !trimmed_key.is_empty() {
                let client_clone = client.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    info!("Attempting session verification using MATRIX_RECOVERY_KEY...");
                    match client_clone.encryption().recovery().recover(&trimmed_key).await {
                        Ok(_) => info!("Successfully verified bot session via Recovery Key! 🎉"),
                        Err(err) => warn!("Failed to verify session using Recovery Key: {:?}", err),
                    }
                });
            }
        }

        Ok(client)
    }
}
