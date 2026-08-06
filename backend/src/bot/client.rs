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
    pub async fn build_and_login_with_token_handles(
        config: &Config,
        token_lock: Option<std::sync::Arc<tokio::sync::RwLock<Option<String>>>>,
        client_lock: Option<std::sync::Arc<tokio::sync::RwLock<Option<Client>>>>,
    ) -> Result<Client> {
        let homeserver_input = &config.homeserver;
        let clean_domain = homeserver_input
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_end_matches('/');

        // Ensure data directory exists for matrix-sdk sqlite store
        tokio::fs::create_dir_all("data").await.ok();

        let store_path = "data/bot_store";

        let mut client = if let Ok(server_name) = ServerName::parse(clean_domain) {
            if let Ok(c) = Client::builder().server_name(&server_name).sqlite_store(store_path, None).build().await {
                c
            } else {
                let fallback_url = if homeserver_input.starts_with("http") { homeserver_input.to_string() } else { format!("https://{}", clean_domain) };
                Client::builder().homeserver_url(&fallback_url).sqlite_store(store_path, None).build().await.context("Failed to build client")?
            }
        } else {
            let fallback_url = if homeserver_input.starts_with("http") { homeserver_input.to_string() } else { format!("https://{}", clean_domain) };
            Client::builder().homeserver_url(&fallback_url).sqlite_store(store_path, None).build().await.context("Failed to build client")?
        };

        if let Some(ref lock) = client_lock {
            *lock.write().await = Some(client.clone());
        }

        if client.matrix_auth().session().is_none() {
            info!("No active Matrix session found in store at {}. Logging in as user: {}", store_path, config.username);

            let user_id = if config.username.starts_with('@') {
                UserId::parse(&config.username)
                    .context("Failed to parse MATRIX_USERNAME as a valid Matrix User ID")?
            } else {
                let raw_id = format!("@{}:{}", config.username, clean_domain);
                UserId::parse(&raw_id).context("Failed to construct valid Matrix User ID")?
            };

            match client
                .matrix_auth()
                .login_username(&user_id, &config.password)
                .initial_device_display_name("Rust Matrix Bot")
                .send()
                .await
            {
                Ok(_) => {
                    info!("Successfully logged in as {}", client.user_id().unwrap());
                }
                Err(err) => {
                    warn!("Failed to login with existing crypto store: {:?}. Resetting store for new device login...", err);
                    tokio::fs::remove_dir_all(store_path).await.ok();

                    let fresh_client = if let Ok(server_name) = ServerName::parse(clean_domain) {
                        if let Ok(c) = Client::builder().server_name(&server_name).sqlite_store(store_path, None).build().await {
                            c
                        } else {
                            let fallback_url = if homeserver_input.starts_with("http") { homeserver_input.to_string() } else { format!("https://{}", clean_domain) };
                            Client::builder().homeserver_url(&fallback_url).sqlite_store(store_path, None).build().await.context("Failed rebuild fallback")?
                        }
                    } else {
                        let fallback_url = if homeserver_input.starts_with("http") { homeserver_input.to_string() } else { format!("https://{}", clean_domain) };
                        Client::builder().homeserver_url(&fallback_url).sqlite_store(store_path, None).build().await.context("Failed rebuild fallback")?
                    };

                    fresh_client
                        .matrix_auth()
                        .login_username(&user_id, &config.password)
                        .initial_device_display_name("Rust Matrix Bot")
                        .send()
                        .await
                        .context("Failed to log in to Matrix server")?;

                    client = fresh_client;
                    if let Some(ref lock) = client_lock {
                        *lock.write().await = Some(client.clone());
                    }
                    info!("Successfully logged in as {}", client.user_id().unwrap());
                }
            }
        } else {
            info!("Restored existing session from store for user: {}", client.user_id().unwrap());
        }

        if let Some(tok) = client.access_token() {
            std::fs::create_dir_all("data").ok();
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                let mut options = std::fs::OpenOptions::new();
                options.write(true).create(true).truncate(true).mode(0o600);
                if let Ok(mut f) = options.open("data/matrix_token.json") {
                    use std::io::Write;
                    let _ = f.write_all(tok.as_bytes());
                }
            }
            #[cfg(not(unix))]
            {
                std::fs::write("data/matrix_token.json", tok).ok();
            }
        }

        if let Some(ref lock) = token_lock {
            if let Some(tok) = client.access_token() {
                *lock.write().await = Some(tok.to_string());
            }
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
