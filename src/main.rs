use anyhow::Result;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod bot;
mod config;
mod db;
mod error;
mod modules;
mod utils;

use bot::MatrixBot;
use config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,matrix_hello_bot=debug,matrix_sdk=info")),
        )
        .init();

    info!("Starting Modular Production Matrix Hello Bot...");

    let config = Config::from_env()?;
    let bot = MatrixBot::new(&config).await?;
    bot.run().await?;

    Ok(())
}
