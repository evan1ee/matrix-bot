use crate::db::DbPool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::webhook::models::WebhookSeverity;
use crate::modules::webhook::renderer::render_alert_message;
use crate::modules::webhook::repository::WebhookRepository;
use crate::modules::webhook::scheduler::notify_outbox;
use crate::utils::html::html_escape;

pub async fn handle_webhook_command(
    pool: &DbPool,
    room_id: &str,
    owner_id: &str,
    sender_name: &str,
    clean_body: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    let clean_lower = clean_body.to_lowercase();

    if !(clean_lower == "webhook"
        || clean_lower == "!webhook"
        || clean_lower.starts_with("webhook ")
        || clean_lower.starts_with("!webhook "))
    {
        return None;
    }

    let parts: Vec<&str> = clean_body.split_whitespace().collect();
    let sub = parts.get(1).map(|s| s.to_lowercase());

    match sub.as_deref() {
        Some("list") | None | Some("") => handle_list(pool, owner_id, sender_name).await,
        Some("create") | Some("add") => {
            let name = parts.get(2).unwrap_or(&"Unnamed Webhook");
            let target_room = parts.get(3).unwrap_or(&room_id);
            handle_create(pool, owner_id, name, target_room, broadcaster).await
        }
        Some("revoke") | Some("delete") | Some("rm") => {
            if let Some(id) = parts.get(2) {
                handle_revoke(pool, owner_id, id, broadcaster).await
            } else {
                Some("⚠️ Usage: <code>!webhook revoke &lt;webhook_id&gt;</code> (or <code>!webhook rm &lt;id&gt;</code>)".to_string())
            }
        }
        Some("test") | Some("ping") => {
            if let Some(id) = parts.get(2) {
                handle_test(pool, owner_id, id, broadcaster).await
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

async fn handle_list(pool: &DbPool, owner_id: &str, sender_name: &str) -> Option<String> {
    match WebhookRepository::list_by_owner(pool, owner_id).await {
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
    pool: &DbPool,
    owner_id: &str,
    name: &str,
    default_room_id: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    match WebhookRepository::create(pool, owner_id, name, default_room_id).await {
        Ok((webhook, raw_token)) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::WebhookCreated(webhook.clone()));
            }

            Some(format!(
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
            ))
        }
        Err(err) => {
            error!("Failed creating webhook: {:?}", err);
            Some("❌ Failed to create webhook in database.".to_string())
        }
    }
}

async fn handle_revoke(
    pool: &DbPool,
    owner_id: &str,
    id: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    match WebhookRepository::delete(pool, id, owner_id).await {
        Ok(true) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::WebhookDeleted {
                    id: id.to_string(),
                    owner_id: owner_id.to_string(),
                });
            }
            Some(format!(
                "🗑️ Webhook <code>{}</code> has been revoked.",
                html_escape(id)
            ))
        }
        Ok(false) => Some(format!(
            "⚠️ Webhook <code>{}</code> not found.",
            html_escape(id)
        )),
        Err(err) => {
            error!("Failed revoking webhook {}: {:?}", id, err);
            Some("❌ Database error revoking webhook.".to_string())
        }
    }
}

async fn handle_test(
    pool: &DbPool,
    owner_id: &str,
    id: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    match WebhookRepository::find_by_id(pool, id, owner_id).await {
        Ok(Some(webhook)) => {
            let (html, plain) = render_alert_message(
                &webhook.name,
                Some("Synthetic Test Ping"),
                "This is a test notification triggered from chat via <code>!webhook test</code>.",
                WebhookSeverity::Info,
                Some("markdown"),
            );

            match WebhookRepository::enqueue_outbox(
                pool,
                &webhook.id,
                owner_id,
                &webhook.default_room_id,
                "info",
                Some("Synthetic Test Ping"),
                &html,
                &plain,
            )
            .await
            {
                Ok(item) => {
                    notify_outbox();
                    if let Some(b) = broadcaster {
                        b.send(SystemEvent::WebhookTriggered {
                            id: webhook.id.clone(),
                            owner_id: owner_id.to_string(),
                            outbox_id: item.id.clone(),
                            room_id: webhook.default_room_id.clone(),
                            severity: "info".to_string(),
                        });
                    }
                    Some(format!(
                        "🚀 Test alert queued for <b>{}</b> to room <code>{}</code>.",
                        html_escape(&webhook.name),
                        html_escape(&webhook.default_room_id)
                    ))
                }
                Err(err) => {
                    error!("Failed enqueueing test alert: {:?}", err);
                    Some("❌ Failed to queue test alert in outbox.".to_string())
                }
            }
        }
        Ok(None) => Some(format!(
            "⚠️ Webhook <code>{}</code> not found.",
            html_escape(id)
        )),
        Err(err) => {
            error!("Failed finding webhook for test: {:?}", err);
            Some("❌ Database error retrieving webhook.".to_string())
        }
    }
}
