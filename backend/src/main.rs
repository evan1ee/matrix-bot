use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod api;
mod bot;
mod config;
mod db;
mod error;
mod events;
mod modules;
mod utils;

use bot::MatrixBot;
use config::Config;
use db::Database;
use events::EventBroadcaster;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,matrix_bot=debug,matrix_sdk=info")),
        )
        .init();

    info!("Starting Modular Production Matrix Bot & Web API...");

    let config = Config::from_env()?;
    config.validate()?;
    let db = Database::connect(&config.database_url, config.db_max_connections).await?;
    let broadcaster = EventBroadcaster::default();

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    // 1. Start background Axum REST API & Web Server with graceful shutdown
    let (matrix_token_handle, matrix_client_handle) =
        api::start_api_server(&config, db.clone(), broadcaster.clone(), shutdown_rx).await;

    // 2. Start Matrix Bot Event Loop with automatic reconnection & retry
    let config_clone = config.clone();
    let db_clone = db.clone();
    let broadcaster_clone = broadcaster.clone();
    let matrix_token_clone = matrix_token_handle.clone();
    let matrix_client_clone = matrix_client_handle.clone();
    let mut bot_shutdown_rx = shutdown_tx.subscribe();

    tokio::spawn(async move {
        loop {
            info!("Starting Matrix bot event loop sync...");
            tokio::select! {
                _ = bot_shutdown_rx.recv() => {
                    info!("Matrix bot loop shutting down cleanly...");
                    break;
                }
                bot_res = MatrixBot::new_full(&config_clone, db_clone.clone(), Some(broadcaster_clone.clone()), Some(matrix_token_clone.clone()), Some(matrix_client_clone.clone())) => {
                    match bot_res {
                        Ok(bot) => {
                            if let Err(err) = bot.run().await {
                                tracing::error!("Matrix bot sync error: {:?}. Retrying in 5 seconds...", err);
                            }
                        }
                        Err(err) => {
                            tracing::error!("Failed to initialize Matrix bot client: {:?}. Retrying in 5 seconds...", err);
                        }
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    });

    tokio::signal::ctrl_c().await?;
    info!("Shutting down Matrix Bot & Web API server...");
    let _ = shutdown_tx.send(());
    // Short grace period for cleanup
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(())
}
