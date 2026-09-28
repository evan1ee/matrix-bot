//! Matrix chat command handler and background delivery worker for webhooks.

use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::Utc;
use matrix_sdk::ruma::{events::room::message::RoomMessageEventContent, RoomId};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};

use crate::application::AppServices;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::domain::webhooks::{
    renderer::render_alert_message, retry_delay, WebhookOutboxItem, WebhookSeverity,
};
use crate::events::EventBroadcaster;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp, ModuleContext};
use crate::utils::html::html_escape;

pub struct WebhookModule;

impl Default for WebhookModule {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for WebhookModule {
    fn name(&self) -> &str {
        "Webhook"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "webhook".to_string(),
                usage: "!webhook list".to_string(),
                description: "List registered webhook endpoints".to_string(),
            },
            CommandHelp {
                name: "webhook create".to_string(),
                usage: "!webhook create <name> [room_id]".to_string(),
                description: "Create an inbound webhook for server alerts".to_string(),
            },
            CommandHelp {
                name: "webhook test".to_string(),
                usage: "!webhook test <id>".to_string(),
                description: "Send a test notification alert".to_string(),
            },
            CommandHelp {
                name: "webhook revoke".to_string(),
                usage: "!webhook revoke <id>".to_string(),
                description: "Revoke and delete a webhook".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        let cmd = ctx.command.to_lowercase();
        if cmd != "webhook" {
            return None;
        }

        let parts: Vec<&str> = ctx.raw_args.split_whitespace().collect();
        let sub = parts.first().map(|s| s.to_lowercase());

        match sub.as_deref() {
            Some("list") | None | Some("") => {
                handle_list(services, &ctx.owner_id, &ctx.sender_name).await
            }
            Some("create") | Some("add") => {
                let name = parts.get(1).copied().unwrap_or("Unnamed Webhook");
                let target_room = parts.get(2).copied().unwrap_or(ctx.room_id.as_str());
                handle_create(services, &ctx.owner_id, name, target_room).await
            }
            Some("revoke") | Some("delete") | Some("rm") => {
                if let Some(id) = parts.get(1) {
                    handle_revoke(services, &ctx.owner_id, id).await
                } else {
                    Some("⚠️ Usage: <code>!webhook revoke &lt;webhook_id&gt;</code> (or <code>!webhook rm &lt;id&gt;</code>)".to_string())
                }
            }
            Some("test") | Some("ping") => {
                if let Some(id) = parts.get(1) {
                    handle_test(services, &ctx.owner_id, id).await
                } else {
                    Some("⚠️ Usage: <code>!webhook test &lt;webhook_id&gt;</code>".to_string())
                }
            }
            _ => Some(
                "🔌 <b>Webhook Integration Commands:</b><br/>\
                 • <code>!webhook list</code> — List your registered webhooks<br/>\
                 • <code>!webhook create &lt;name&gt; [room_id]</code> — Register a new webhook<br/>\
                 • <code>!webhook test &lt;id&gt;</code> — Send a test notification alert<br/>\
                 • <code>!webhook revoke &lt;id&gt;</code> — Delete and revoke a webhook"
                    .to_string(),
            ),
        }
    }

    async fn spawn_background_workers(
        &self,
        ctx: &ModuleContext,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        vec![start_webhook_scheduler(ctx.clone())]
    }
}

async fn handle_list(
    services: &AppServices,
    owner_id: &AccountId,
    sender_name: &str,
) -> Option<String> {
    match services.webhooks.list(owner_id).await {
        Ok(hooks) => {
            if hooks.is_empty() {
                Some("🔌 You have no active webhooks. Create one with <code>!webhook create &lt;name&gt;</code> or in the Web UI.".to_string())
            } else {
                let mut html = format!(
                    "🔌 <b>Registered Webhooks for {}:</b><br/>",
                    html_escape(sender_name)
                );
                for h in hooks {
                    let status = if h.is_active {
                        "🟢 Active"
                    } else {
                        "⚪ Inactive"
                    };
                    html.push_str(&format!(
                        "• [<code>{}</code>] <b>{}</b> &mdash; {}<br/>\
                         &nbsp;&nbsp;Room: <code>{}</code> | Delivered: <b>{}</b> | Failures: <b>{}</b><br/>",
                        html_escape(&h.id),
                        html_escape(&h.name),
                        status,
                        html_escape(&h.default_room_id),
                        h.total_delivered_count,
                        h.failure_count
                    ));
                }
                Some(html)
            }
        }
        Err(err) => {
            error!("Failed to list webhooks for {}: {:?}", owner_id, err);
            Some("❌ Failed to query webhooks from database.".to_string())
        }
    }
}

async fn handle_create(
    services: &AppServices,
    owner_id: &AccountId,
    name: &str,
    default_room_id: &str,
) -> Option<String> {
    match services.webhooks.create(owner_id, name, default_room_id).await {
        Ok((webhook, raw_token)) => Some(format!(
            "✅ <b>Webhook Created Successfully!</b><br/>\
             • <b>ID:</b> <code>{}</code><br/>\
             • <b>Name:</b> {}<br/>\
             • <b>Target Room:</b> <code>{}</code><br/>\
             • <b>Secret Token:</b> <code>{}</code><br/><br/>\
             ⚠️ <i>Copy this token now. It will not be shown again.</i><br/><br/>\
             <b>Example Usage:</b><br/>\
             <code>curl -X POST https://&lt;bot-host&gt;/api/v1/webhooks/{} -d \"Server alert message\"</code>",
            html_escape(&webhook.id),
            html_escape(&webhook.name),
            html_escape(&webhook.default_room_id),
            html_escape(&raw_token),
            html_escape(&raw_token)
        )),
        Err(err) => {
            error!("Failed creating webhook: {:?}", err);
            Some("❌ Failed to create webhook in database.".to_string())
        }
    }
}

async fn handle_revoke(services: &AppServices, owner_id: &AccountId, id: &str) -> Option<String> {
    match services.webhooks.delete(owner_id, id).await {
        Ok(()) => Some(format!(
            "🗑️ Webhook <code>{}</code> has been revoked.",
            html_escape(id)
        )),
        Err(crate::application::AppError::NotFound(_)) => Some(format!(
            "⚠️ Webhook <code>{}</code> not found.",
            html_escape(id)
        )),
        Err(err) => {
            error!("Failed revoking webhook {}: {:?}", id, err);
            Some("❌ Database error revoking webhook.".to_string())
        }
    }
}

async fn handle_test(services: &AppServices, owner_id: &AccountId, id: &str) -> Option<String> {
    match services.webhooks.get(owner_id, id).await {
        Ok(webhook) => {
            let (html, plain) = render_alert_message(
                &webhook.name,
                Some("Synthetic Test Ping"),
                "This is a test notification triggered from chat via <code>!webhook test</code>.",
                WebhookSeverity::Info,
                Some("markdown"),
            );

            match services
                .webhooks
                .enqueue_alert(
                    &webhook,
                    &webhook.default_room_id,
                    Some("Synthetic Test Ping"),
                    &html,
                    &plain,
                    WebhookSeverity::Info,
                )
                .await
            {
                Ok(_) => Some(format!(
                    "🚀 Test alert queued for <b>{}</b> to room <code>{}</code>.",
                    html_escape(&webhook.name),
                    html_escape(&webhook.default_room_id)
                )),
                Err(err) => {
                    error!("Failed enqueueing test alert: {:?}", err);
                    Some("❌ Failed to queue test alert in outbox.".to_string())
                }
            }
        }
        Err(crate::application::AppError::NotFound(_)) => Some(format!(
            "⚠️ Webhook <code>{}</code> not found.",
            html_escape(id)
        )),
        Err(err) => {
            error!("Failed finding webhook for test: {:?}", err);
            Some("❌ Database error retrieving webhook.".to_string())
        }
    }
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
    let is_auth = crate::interface::chat::rooms::is_room_authorized_for_account(
        &ctx.services.rooms,
        Some(&ctx.client),
        &item.owner_id,
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
    let notify = ctx.services.webhooks.get_outbox_notify();

    tokio::spawn(async move {
        info!("Starting Webhook Outbox Delivery Worker");
        let sweep_interval = StdDuration::from_secs(3);

        loop {
            // Drain all available pending items in batches
            loop {
                let pending = match ctx.services.webhooks.fetch_pending_outbox(20).await {
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
                            if let Err(err) = ctx
                                .services
                                .webhooks
                                .mark_outbox_delivered(&item.id, &item.webhook_id)
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
                                let _ = ctx
                                    .services
                                    .webhooks
                                    .mark_outbox_failed(&item.id, &item.webhook_id, &err_msg)
                                    .await;
                            } else {
                                let backoff = retry_delay(item.attempts);
                                let next_attempt_at = Utc::now() + backoff;
                                warn!(
                                    "Webhook delivery failed (attempt {}/{}), retrying in {:?}: {}",
                                    next_attempts, item.max_attempts, backoff, err_msg
                                );
                                let _ = ctx
                                    .services
                                    .webhooks
                                    .mark_outbox_retry(
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
