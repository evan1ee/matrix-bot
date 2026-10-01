//! The Matrix sync loop: build a client, sync until it fails, reconnect with
//! backoff — until shutdown.

use anyhow::Result;
use chrono::{DateTime, Utc};
use matrix_sdk::config::SyncSettings;
use matrix_sdk::{Client, LoopCtrl};
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::interface::channels::inbound::Inbound;
use crate::interface::channels::matrix::adapter::MatrixAdapter;
use crate::interface::channels::matrix::client::BotClient;
use crate::interface::channels::matrix::handler::register_handlers;
use crate::interface::channels::matrix::verification::{
    register_verification_handlers, VerificationPolicy,
};

/// Reconnect backoff bounds for the Matrix sync loop.
const RECONNECT_MIN: Duration = Duration::from_secs(5);
const RECONNECT_MAX: Duration = Duration::from_secs(300);

/// Whether this generation should reconnect at the minimum backoff rather
/// than doubling from wherever it left off.
///
/// Compares the last recorded sync before and after the generation: if a new
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

/// Logs in (restoring or creating the session; `ensure_self_verified` runs on
/// every boot inside `build_and_login_with_token_handles`) and answers
/// verification requests, so the bot's session can be signed into another
/// user's trust; without that its sessions stay Unverified.
async fn connect(adapter: &MatrixAdapter) -> Result<Client> {
    let handles = &adapter.handles;
    let client = BotClient::build_and_login_with_token_handles(
        &adapter.config,
        Some(handles.access_token.clone()),
        Some(handles.client.clone()),
    )
    .await?;

    *handles.client.write().await = Some(client.clone());

    let own_user = client
        .user_id()
        .map(|u| u.to_string())
        .unwrap_or_else(|| adapter.config.username.clone());
    register_verification_handlers(
        &client,
        VerificationPolicy::new(own_user, adapter.config.verification_allowlist.clone()),
    );
    Ok(client)
}

/// One sync generation: register the handlers, publish the client and sync
/// until it fails.
async fn sync_generation(
    adapter: &MatrixAdapter,
    inbound: &Arc<Inbound>,
    client: Client,
) -> Result<()> {
    register_handlers(
        &client,
        &adapter.config,
        inbound.clone(),
        adapter.db.clone(),
        adapter.services.clone(),
    );

    // Job handlers send through this generation's client while it syncs;
    // dropping the guard (any return path, including cancellation of this
    // future) hands them `None`, so their jobs defer instead of failing.
    let _published = adapter.handles.matrix.publish(client.clone());

    info!("Matrix Bot is active and listening for events...");

    // `sync_with_callback` (rather than plain `sync`) is the hook `/health/ready`
    // needs: it runs the callback after every successful response, which is
    // this loop's only clean signal that "a sync completed" — the point at
    // which the client's local state is caught up with the homeserver.
    let sync_health = adapter.handles.sync_health.clone();
    client
        .sync_with_callback(SyncSettings::default(), move |_response| {
            sync_health.record_sync(Utc::now());
            async { LoopCtrl::Continue }
        })
        .await?;

    Ok(())
}

/// Runs the Matrix sync loop until `shutdown` is cancelled.
///
/// Every await that can block for a long time — building the client, syncing,
/// and backing off — sits inside a `select!` against the shutdown token. An
/// earlier version awaited `bot.run()` in a select *branch body*, which is
/// outside the select, so once syncing started Ctrl-C could not interrupt it and
/// the process simply exited from under the running task.
///
/// Returning drops the in-flight `sync_generation` future, and with it that
/// generation's `PublishedClient` guard, which hands the job handlers and the
/// events dispatcher `None` until the next generation publishes its client.
pub async fn run_bot_loop(
    adapter: &MatrixAdapter,
    inbound: Arc<Inbound>,
    shutdown: CancellationToken,
) {
    let handles = &adapter.handles;
    let mut backoff = RECONNECT_MIN;

    loop {
        info!("Starting Matrix bot event loop sync...");

        let bot = tokio::select! {
            _ = shutdown.cancelled() => {
                info!("Matrix bot loop shutting down cleanly...");
                return;
            }
            built = connect(adapter) => built,
        };

        match bot {
            Ok(client) => {
                let sync_before = handles.sync_health.last_sync();

                let outcome = tokio::select! {
                    _ = shutdown.cancelled() => {
                        info!("Matrix bot loop shutting down cleanly...");
                        return;
                    }
                    outcome = sync_generation(adapter, &inbound, client) => outcome,
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
