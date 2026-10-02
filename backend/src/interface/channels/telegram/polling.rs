//! The `getUpdates` long-polling loop (polling mode, T11).

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use teloxide_core::payloads::GetUpdatesSetters;
use teloxide_core::prelude::*;
use teloxide_core::types::{AllowedUpdate, Update};
use teloxide_core::{ApiError, RequestError};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::adapter::retry_delay;
use super::ingress::TelegramIngress;
use super::outbound::{bounded, redact};

/// How long Telegram holds an empty `getUpdates` open.
pub const POLL_TIMEOUT_SECS: u32 = 50;
/// The whole call: the long poll plus a margin for the network.
pub const POLL_CALL_TIMEOUT: Duration = Duration::from_secs(65);

/// The next `offset` after seeing `id`: one past the highest id so far, so a
/// dropped or failing update is never fetched again. Saturates at `i32::MAX`
/// (the Bot API's offset type).
pub fn next_offset(current: i32, id: u32) -> i32 {
    let after = i32::try_from(u64::from(id) + 1).unwrap_or(i32::MAX);
    current.max(after)
}

/// An update as it travels the queue: the JSON the workers parse again.
pub fn to_queue_item(update: &Update) -> Result<Bytes, serde_json::Error> {
    serde_json::to_vec(update).map(Bytes::from)
}

/// Whether polling can never succeed: a refused token.
fn is_fatal(err: &RequestError) -> bool {
    matches!(err, RequestError::Api(ApiError::InvalidToken))
}

/// Polls until `cancel` fires or Telegram refuses the token. Errors back off
/// (capped, never spinning); every received update advances the offset.
pub(super) async fn poll(
    bot: &Bot,
    ingress: &TelegramIngress,
    connected: &AtomicBool,
    cancel: &CancellationToken,
) -> anyhow::Result<()> {
    let mut attempt = 0u32;
    let mut offset = 0i32;
    // A webhook and `getUpdates` exclude each other (409 Conflict).
    loop {
        match bounded(bot.delete_webhook().send()).await {
            Ok(_) => break,
            Err(err) => {
                warn!(error = %err, "Could not delete the Telegram webhook");
                if !backoff(&mut attempt, cancel).await {
                    return Ok(());
                }
            }
        }
    }
    attempt = 0;
    loop {
        let request = bot
            .get_updates()
            .offset(offset)
            .timeout(POLL_TIMEOUT_SECS)
            .allowed_updates([AllowedUpdate::Message, AllowedUpdate::CallbackQuery]);
        let result = tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            result = tokio::time::timeout(POLL_CALL_TIMEOUT, request.send()) => result,
        };
        match result {
            Ok(Ok(updates)) => {
                attempt = 0;
                if !connected.swap(true, Ordering::Relaxed) {
                    info!("Telegram long polling started");
                }
                for update in updates {
                    offset = next_offset(offset, update.id.0);
                    let item = match to_queue_item(&update) {
                        Ok(item) => item,
                        Err(err) => {
                            warn!(error = %err, "Could not queue a Telegram update");
                            continue;
                        }
                    };
                    tokio::select! {
                        _ = cancel.cancelled() => return Ok(()),
                        pushed = ingress.push_wait(item) => {
                            if pushed.is_err() {
                                return Err(anyhow::anyhow!("the Telegram update queue closed"));
                            }
                        }
                    }
                }
            }
            Ok(Err(err)) if is_fatal(&err) => {
                return Err(anyhow::anyhow!(
                    "Telegram refused the bot token (getUpdates)"
                ));
            }
            Ok(Err(err)) => {
                connected.store(false, Ordering::Relaxed);
                warn!(error = %redact(&err.to_string()), "Telegram getUpdates failed");
                if !backoff(&mut attempt, cancel).await {
                    return Ok(());
                }
            }
            Err(_) => {
                connected.store(false, Ordering::Relaxed);
                warn!("Telegram getUpdates timed out");
                if !backoff(&mut attempt, cancel).await {
                    return Ok(());
                }
            }
        }
    }
}

/// Sleeps the next retry delay; `false` when cancelled meanwhile.
pub(super) async fn backoff(attempt: &mut u32, cancel: &CancellationToken) -> bool {
    let delay = retry_delay(*attempt);
    *attempt = attempt.saturating_add(1);
    tokio::select! {
        _ = cancel.cancelled() => false,
        _ = tokio::time::sleep(delay) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use teloxide_core::types::Update;

    #[test]
    fn the_offset_moves_past_the_highest_id_seen() {
        assert_eq!(next_offset(0, 10), 11);
        assert_eq!(next_offset(11, 5), 11);
        assert_eq!(next_offset(11, 11), 12);
    }

    #[test]
    fn the_offset_saturates_instead_of_overflowing() {
        assert_eq!(next_offset(0, u32::MAX), i32::MAX);
        assert_eq!(next_offset(i32::MAX, 3), i32::MAX);
    }

    #[test]
    fn a_polled_update_survives_the_trip_through_the_queue() {
        for fixture in [
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../contracts/channels/telegram_private_text.json"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../contracts/channels/telegram_group_command.json"
            )),
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../contracts/channels/telegram_callback_query.json"
            )),
        ] {
            let update: Update = serde_json::from_str(fixture).unwrap();
            let bytes = to_queue_item(&update).unwrap();
            let back: Update = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(back, update);
        }
    }

    #[test]
    fn long_polling_waits_fifty_seconds_inside_a_longer_call_bound() {
        assert_eq!(POLL_TIMEOUT_SECS, 50);
        assert!(POLL_CALL_TIMEOUT > Duration::from_secs(u64::from(POLL_TIMEOUT_SECS)));
    }
}
