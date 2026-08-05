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

    // 1. Start background Axum REST API & Web Server
    api::start_api_server(&config, db.clone(), broadcaster.clone()).await;

    // 2. Start Matrix Bot Event Loop with event broadcaster
    let bot = MatrixBot::new_with_db_and_broadcaster(&config, db, Some(broadcaster)).await?;
    bot.run().await?;

    Ok(())
}
