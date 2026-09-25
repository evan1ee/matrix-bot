use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use matrix_bot::config::Config;
use matrix_bot::db::Database;

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

/// Cancels `shutdown` on SIGINT or SIGTERM.
///
/// Registered before boot, so a signal during a slow database connect is not
/// lost. If waiting for Ctrl-C fails, shutting down is the only safe outcome:
/// the process could otherwise no longer be stopped gracefully.
///
/// A second Ctrl-C does not force-quit: this handler task ends after firing
/// `shutdown.cancel()` once, so nothing here is listening for a repeat.
/// That is deliberate rather than an oversight — `app::SHUTDOWN_GRACE` (15 s)
/// already bounds how long shutdown can take, aborting any laggard, so a
/// force-quit path would only risk skipping the pool close for no benefit.
fn cancel_on_signal(shutdown: CancellationToken) -> Result<()> {
    #[cfg(unix)]
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;

    tokio::spawn(async move {
        #[cfg(unix)]
        let signal = tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = term.recv() => Ok(()),
        };
        #[cfg(not(unix))]
        let signal = tokio::signal::ctrl_c().await;

        match signal {
            Ok(()) => info!("Received SIGINT/SIGTERM..."),
            Err(err) => error!("Failed to listen for Ctrl-C, shutting down: {err:?}"),
        }
        shutdown.cancel();
    });
    Ok(())
}

fn main() -> Result<()> {
    let runtime = tokio::runtime::Runtime::new().context("failed to start the Tokio runtime")?;
    let result = runtime.block_on(async_main());
    // Every task `app::run` owns has stopped or been aborted by now. Dropping
    // the runtime would instead wait, unbounded, for in-flight blocking-pool
    // work, which would break the 15 s shutdown bound (`app::SHUTDOWN_GRACE`).
    runtime.shutdown_background();
    result
}

async fn async_main() -> Result<()> {
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

    let shutdown = CancellationToken::new();
    cancel_on_signal(shutdown.clone())?;
    matrix_bot::app::run(config, shutdown).await
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
