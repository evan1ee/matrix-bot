// matrix-sdk 0.18's crypto futures nest deeper than the default limit of 128
// when rustc proves the spawned sync task is `Send`; rustc's own suggestion.
#![recursion_limit = "256"]

use anyhow::{Context, Result};
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
    handles: matrix_bot::bot::handles::BotHandles,
    mut shutdown: broadcast::Receiver<()>,
) {
    let mut backoff = RECONNECT_MIN;

    loop {
        info!("Starting Matrix bot event loop sync...");

        let build = MatrixBot::new_full(
            &config,
            db.clone(),
            Some(broadcaster.clone()),
            handles.clone(),
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

/// Loads `.env` from the given path only -- never dotenvy's own upward
/// directory search, which would silently pick up a `.env` from a parent
/// directory (e.g. the main checkout's real production credentials, when run
/// from a worktree under it). A missing file is fine (env vars may be set
/// another way); any other error (e.g. a malformed file) fails startup so a
/// broken `.env` is never silently ignored.
fn load_dotenv_from(path: &std::path::Path) -> Result<()> {
    match dotenvy::from_path(path) {
        Ok(()) => Ok(()),
        Err(dotenvy::Error::Io(err)) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("failed to load {}", path.display())),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Read only ./.env -- run from the repo root. See load_dotenv_from's doc
    // comment for why this must not search parent directories.
    load_dotenv_from(std::path::Path::new(".env"))?;

    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && args[1] == "mcp" {
        // Direct logs to stderr so stdout is purely reserved for JSON-RPC messages
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| EnvFilter::new("warn,matrix_bot=info")),
            )
            .init();

        let database_url = matrix_bot::config::database_url_from_env()?;
        let db = Database::connect(&database_url, 5).await?;

        let mut owner_id = String::new();
        let mut i = 2;
        while i < args.len() {
            if (args[i] == "--owner" || args[i] == "--account") && i + 1 < args.len() {
                owner_id = args[i + 1].clone();
                i += 2;
            } else {
                i += 1;
            }
        }

        if owner_id.is_empty() {
            anyhow::bail!(
                "MCP mode needs an explicit account: pass --owner <account id>. \
                 It used to default to the oldest account in the database, which made \
                 an authorisation decision by accident."
            );
        }

        matrix_bot::modules::mcp::stdio::run_stdio(&db.pool, &owner_id).await?;
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,matrix_bot=debug,matrix_sdk=info")),
        )
        .init();

    info!("Starting Modular Production Matrix Bot & Web API...");

    let config = Config::from_env()?;
    config.validate()?;

    if config.registration_secret.is_some() {
        info!("Registration mode: INVITE-ONLY (invite secret configured)");
    } else if config.registration_open {
        info!("Registration mode: OPEN (open registration enabled)");
    } else {
        info!("Registration mode: CLOSED (release default: open internet scanners cannot register accounts)");
    }
    let db = Database::connect(&config.database_url, config.db_max_connections).await?;
    let broadcaster = EventBroadcaster::default();
    let handles = matrix_bot::bot::handles::BotHandles::default();

    let (shutdown_tx, shutdown_rx) = broadcast::channel::<()>(1);

    // 1. Start background Axum REST API & Web Server with graceful shutdown
    api::start_api_server(
        &config,
        db.clone(),
        broadcaster.clone(),
        handles.clone(),
        shutdown_rx,
    )
    .await?;

    // 2. Start Matrix Bot Event Loop with automatic reconnection & retry
    let bot_task = tokio::spawn(run_bot_loop(
        config.clone(),
        db.clone(),
        broadcaster.clone(),
        handles,
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

#[cfg(test)]
mod dotenv_tests {
    use super::load_dotenv_from;

    #[test]
    fn missing_file_is_not_an_error() {
        let path = std::path::Path::new("this-file-does-not-exist.env");
        assert!(load_dotenv_from(path).is_ok());
    }

    #[test]
    fn malformed_file_is_an_error() {
        let dir =
            std::env::temp_dir().join(format!("matrix-bot-dotenv-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(".env");
        // No `=` -- dotenvy rejects this as a parse error.
        std::fs::write(&path, "NOT_A_VALID_LINE\n").unwrap();

        let result = load_dotenv_from(&path);

        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_err());
    }
}
