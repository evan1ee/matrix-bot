use anyhow::Result;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use matrix_bot::api;
use matrix_bot::bot::MatrixBot;
use matrix_bot::config::Config;
use matrix_bot::db::Database;
use matrix_bot::events::EventBroadcaster;

/// Reconnect backoff bounds for the Matrix sync loop.
const RECONNECT_MIN: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(300);

/// Runs the Matrix sync loop until shutdown is signalled.
///
/// Every await that can block for a long time — building the client, syncing,
/// and backing off — sits inside a `select!` against the shutdown channel. An
/// earlier version awaited `bot.run()` in a select *branch body*, which is
/// outside the select, so once syncing started Ctrl-C could not interrupt it and
/// the process simply exited from under the running task.
async fn run_bot_loop(
    config: Config,
    db: Database,
    broadcaster: EventBroadcaster,
    bot_token_handle: std::sync::Arc<tokio::sync::RwLock<Option<String>>>,
    bot_client_handle: std::sync::Arc<tokio::sync::RwLock<Option<matrix_sdk::Client>>>,
    pending_merges: std::sync::Arc<matrix_bot::bot::linking::PendingMerges>,
    mut shutdown: broadcast::Receiver<()>,
) {
    let mut backoff = RECONNECT_MIN;

    loop {
        info!("Starting Matrix bot event loop sync...");

        let build = MatrixBot::new_full(
            &config,
            db.clone(),
            Some(broadcaster.clone()),
            Some(bot_token_handle.clone()),
            Some(bot_client_handle.clone()),
            Some(pending_merges.clone()),
        );

        let bot = tokio::select! {
            _ = shutdown.recv() => {
                info!("Matrix bot loop shutting down cleanly...");
                return;
            }
            built = build => built,
        };

        match bot {
            Ok(bot) => {
                let outcome = tokio::select! {
                    _ = shutdown.recv() => {
                        info!("Matrix bot loop shutting down cleanly...");
                        return;
                    }
                    outcome = bot.run() => outcome,
                };

                match outcome {
                    // A clean return means sync stopped on its own; reconnect promptly.
                    Ok(()) => backoff = RECONNECT_MIN,
                    Err(err) => error!("Matrix bot sync error: {:?}", err),
                }
            }
            Err(err) => error!("Failed to initialize Matrix bot client: {:?}", err),
        }

        info!("Reconnecting to Matrix in {:?}...", backoff);
        tokio::select! {
            _ = shutdown.recv() => {
                info!("Matrix bot loop shutting down cleanly...");
                return;
            }
            _ = tokio::time::sleep(backoff) => {}
        }

        // Back off on repeated failures so a persistently unreachable homeserver
        // is not hammered every five seconds.
        backoff = (backoff * 2).min(RECONNECT_MAX);
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load .env from the project root (or cwd) before anything reads env vars.
    dotenvy::dotenv().ok();

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

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

    // 1. Start background Axum REST API & Web Server with graceful shutdown
    let (bot_token_handle, bot_client_handle, pending_merges) =
        api::start_api_server(&config, db.clone(), broadcaster.clone(), shutdown_rx).await;

    // 2. Start Matrix Bot Event Loop with automatic reconnection & retry
    let bot_task = tokio::spawn(run_bot_loop(
        config.clone(),
        db.clone(),
        broadcaster.clone(),
        bot_token_handle,
        bot_client_handle,
        pending_merges,
        shutdown_tx.subscribe(),
    ));

    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut term = signal(SignalKind::terminate())?;
        tokio::select! {
            _ = tokio::signal::ctrl_c() => info!("Received SIGINT..."),
            _ = term.recv() => info!("Received SIGTERM..."),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await?;
    }

    info!("Shutting down Matrix Bot & Web API server...");
    let _ = shutdown_tx.send(());

    // Wait for the bot loop to actually observe the signal rather than assuming
    // a fixed grace period is enough.
    match tokio::time::timeout(Duration::from_secs(10), bot_task).await {
        Ok(Ok(())) => info!("Matrix bot loop stopped."),
        Ok(Err(err)) => error!("Matrix bot task panicked: {:?}", err),
        Err(_) => error!("Matrix bot loop did not stop within 10s; exiting anyway."),
    }

    Ok(())
}
