use chrono::{Duration, Utc};
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use std::sync::{Arc, OnceLock};
use std::time::Duration as StdDuration;
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::modules::webhook::models::WebhookOutboxItem;
use crate::modules::webhook::repository::WebhookRepository;
use crate::modules::ModuleContext;

static OUTBOX_NOTIFY: OnceLock<Arc<Notify>> = OnceLock::new();

/// Returns the global notification signal for waking the outbox worker immediately.
pub fn get_outbox_notify() -> Arc<Notify> {
    OUTBOX_NOTIFY
        .get_or_init(|| Arc::new(Notify::new()))
        .clone()
}

/// Triggers an immediate wakeup of the background outbox worker.
pub fn notify_outbox() {
    get_outbox_notify().notify_one();
}

/// Calculates exponential backoff delay based on retry attempt.
pub fn retry_delay(attempts_so_far: i64) -> Duration {
    let exponent = attempts_so_far.clamp(0, 9);
    let base_seconds = 1 << exponent; // 1s, 2s, 4s, 8s, 16s, 32s, 64s, 128s, 256s, 512s
    Duration::seconds(base_seconds)
}

/// Attempts delivery of an outbox item to Matrix.
async fn deliver_outbox_item(ctx: &ModuleContext, item: &WebhookOutboxItem) -> Result<(), String> {
    let room_id = <&RoomId>::try_from(item.room_id.as_str())
        .map_err(|_| format!("Invalid Matrix room ID: {}", item.room_id))?;

    let room = ctx
        .client
        .get_room(room_id)
        .ok_or_else(|| format!("Bot is not joined to room: {}", item.room_id))?;

    // Verify that the webhook owner is authorized for this room before delivery
    let owner_id = crate::domain::accounts::AccountId::from(item.owner_id.as_str());
    let is_auth = crate::interface::chat::rooms::is_room_authorized_for_account(
        &ctx.services.rooms,
        Some(&ctx.client),
        &owner_id,
        &item.room_id,
    )
    .await;
    if !is_auth {
        return Err(format!(
            "Delivery rejected: owner {} is not joined to room {}",
            item.owner_id, item.room_id
        ));
    }

    let content = RoomMessageEventContent::text_html(&item.plain_body, &item.html_body);

    room.send(content)
        .await
        .map_err(|err| format!("Matrix homeserver delivery failed: {err:?}"))?;

    Ok(())
}

/// Starts the background Tokio worker task draining the webhook outbox.
pub fn start_webhook_scheduler(ctx: ModuleContext) -> JoinHandle<()> {
    let notify = get_outbox_notify();

    tokio::spawn(async move {
        info!("Starting Webhook Outbox Delivery Worker");
        let sweep_interval = StdDuration::from_secs(3);

        loop {
            // Drain all available pending items in batches
            loop {
                let pending = match WebhookRepository::fetch_pending_outbox(&ctx.db.pool, 20).await
                {
                    Ok(batch) => batch,
                    Err(err) => {
                        error!("Failed reading webhook outbox batch: {:?}", err);
                        break;
                    }
                };

                if pending.is_empty() {
                    break;
                }

                for item in pending {
                    match deliver_outbox_item(&ctx, &item).await {
                        Ok(_) => {
                            info!("Delivered webhook alert {} to {}", item.id, item.room_id);
                            if let Err(err) = WebhookRepository::mark_outbox_delivered(
                                &ctx.db.pool,
                                &item.id,
                                &item.webhook_id,
                            )
                            .await
                            {
                                error!("Failed to mark outbox item as delivered: {:?}", err);
                            }
                        }
                        Err(err_msg) => {
                            let next_attempts = item.attempts + 1;
                            if next_attempts >= item.max_attempts {
                                warn!(
                                    "Webhook outbox item {} reached max attempts. Marking failed: {}",
                                    item.id, err_msg
                                );
                                let _ = WebhookRepository::mark_outbox_failed(
                                    &ctx.db.pool,
                                    &item.id,
                                    &item.webhook_id,
                                    &err_msg,
                                )
                                .await;
                            } else {
                                let backoff = retry_delay(item.attempts);
                                let next_attempt_at = Utc::now() + backoff;
                                warn!(
                                    "Webhook delivery failed (attempt {}/{}), retrying in {:?}: {}",
                                    next_attempts, item.max_attempts, backoff, err_msg
                                );
                                let _ = WebhookRepository::mark_outbox_retry(
                                    &ctx.db.pool,
                                    &item.id,
                                    &item.webhook_id,
                                    next_attempts,
                                    next_attempt_at,
                                    &err_msg,
                                )
                                .await;
                            }
                        }
                    }
                }
            }

            // Sleep until either signaled by an incoming alert or sweep timeout
            tokio::select! {
                _ = notify.notified() => {}
                _ = tokio::time::sleep(sweep_interval) => {}
            }
        }
    })
}
