//! The Matrix sync loop: build a client, sync until it fails, reconnect with
//! backoff — until shutdown.

use chrono::{DateTime, Utc};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::application::AppServices;
use crate::config::Config;
use crate::db::Database;
use crate::interface::channels::matrix::handles::BotHandles;
use crate::interface::channels::matrix::MatrixBot;

/// Reconnect backoff bounds for the Matrix sync loop.
const RECONNECT_MIN: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(300);

/// Whether this generation should reconnect at the minimum backoff rather
/// than doubling from wherever it left off.
///
/// Compares the last recorded sync before and after `bot.run()`: if a new
/// sync landed during this generation, the connection was healthy for at
/// least a moment before it dropped, so a fresh problem should not inherit
/// a backoff built up from a *previous*, unrelated string of failures. A
/// long-healthy connection hitting one transient error must not sleep up to
/// `RECONNECT_MAX` (300 s) before retrying.
fn should_reset_backoff(
    sync_before: Option<DateTime<Utc>>,
    sync_after: Option<DateTime<Utc>>,
) -> bool {
    match (sync_before, sync_after) {
        (Some(before), Some(after)) => after > before,
        (None, Some(_)) => true,
        _ => false,
    }
}

/// Runs the Matrix sync loop until `shutdown` is cancelled.
///
/// Every await that can block for a long time — building the client, syncing,
/// and backing off — sits inside a `select!` against the shutdown token. An
/// earlier version awaited `bot.run()` in a select *branch body*, which is
/// outside the select, so once syncing started Ctrl-C could not interrupt it and
/// the process simply exited from under the running task.
///
/// Returning drops the in-flight `bot.run()` future, and with it that
/// generation's `PublishedClient` guard, which hands the job handlers and the
/// events dispatcher `None` until the next generation publishes its client.
pub async fn run_bot_loop(
    config: Config,
    db: Database,
    services: AppServices,
    handles: BotHandles,
    shutdown: CancellationToken,
) {
    let mut backoff = RECONNECT_MIN;

    loop {
        info!("Starting Matrix bot event loop sync...");

        let build = MatrixBot::new_full(&config, db.clone(), services.clone(), handles.clone());

        let bot = tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Matrix bot loop shutting down cleanly...");
                return;
            }
            built = build => built,
        };

        match bot {
            Ok(bot) => {
                let sync_before = handles.sync_health.last_sync();

                let outcome = tokio::select! {
                    _ = shutdown.cancelled() => {
                        info!("Matrix bot loop shutting down cleanly...");
                        return;
                    }
                    outcome = bot.run() => outcome,
                };

                let sync_after = handles.sync_health.last_sync();

                match outcome {
                    // A clean return means sync stopped on its own; reconnect promptly.
                    Ok(()) => backoff = RECONNECT_MIN,
                    Err(err) => {
                        error!("Matrix bot sync error: {:?}", err);
                        if should_reset_backoff(sync_before, sync_after) {
                            info!(
                                "This connection completed at least one sync before failing; \
                                 resetting backoff instead of doubling it."
                            );
                            backoff = RECONNECT_MIN;
                        }
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    #[test]
    fn never_synced_before_or_during_does_not_reset() {
        assert!(!should_reset_backoff(None, None));
    }

    #[test]
    fn a_first_sync_this_generation_resets() {
        assert!(should_reset_backoff(None, Some(Utc::now())));
    }

    #[test]
    fn a_newer_sync_than_before_resets() {
        let before = Utc::now();
        let after = before + ChronoDuration::seconds(30);

        assert!(should_reset_backoff(Some(before), Some(after)));
    }

    #[test]
    fn an_unchanged_last_sync_does_not_reset() {
        let last = Utc::now();

        assert!(!should_reset_backoff(Some(last), Some(last)));
    }

    #[test]
    fn no_sync_recorded_after_a_previous_one_does_not_reset() {
        // Should not happen (SyncHealth never forgets), but the decision must
        // stay conservative rather than panic or reset on an impossible input.
        assert!(!should_reset_backoff(Some(Utc::now()), None));
    }
}
