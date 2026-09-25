//! The Matrix sync loop: build a client, sync until it fails, reconnect with
//! backoff — until shutdown.

use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::channels::matrix::handles::BotHandles;
use crate::interface::channels::matrix::MatrixBot;

/// Reconnect backoff bounds for the Matrix sync loop.
const RECONNECT_MIN: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(300);

/// Runs the Matrix sync loop until `shutdown` is cancelled.
///
/// Every await that can block for a long time — building the client, syncing,
/// and backing off — sits inside a `select!` against the shutdown token. An
/// earlier version awaited `bot.run()` in a select *branch body*, which is
/// outside the select, so once syncing started Ctrl-C could not interrupt it and
/// the process simply exited from under the running task.
///
/// Returning drops the in-flight `bot.run()` future, and with it that
/// generation's `BackgroundWorkers` guard, which aborts its scheduler tasks.
pub async fn run_bot_loop(
    config: Config,
    db: Database,
    broadcaster: EventBroadcaster,
    handles: BotHandles,
    shutdown: CancellationToken,
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
            _ = shutdown.cancelled() => {
                info!("Matrix bot loop shutting down cleanly...");
                return;
            }
            built = build => built,
        };

        match bot {
            Ok(bot) => {
                let outcome = tokio::select! {
                    _ = shutdown.cancelled() => {
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
            _ = shutdown.cancelled() => {
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
