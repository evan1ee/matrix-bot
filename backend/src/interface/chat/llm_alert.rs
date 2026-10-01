use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Weak;
use tracing::{debug, error, info, warn};

use crate::application::settings::SettingsService;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::message::{Content, OutgoingMessage, Target, Visibility};
use crate::interface::channels::registry::ChannelRegistry;
use crate::utils::html::html_escape;

static AI_ERROR_ALERTED: AtomicBool = AtomicBool::new(false);

/// Resets the outage alert state when AI succeeds or settings change.
pub fn clear_ai_error_state() {
    AI_ERROR_ALERTED.store(false, Ordering::Relaxed);
}

/// Checks whether an AI outage alert has already been sent.
pub fn is_ai_error_alerted() -> bool {
    AI_ERROR_ALERTED.load(Ordering::Relaxed)
}

/// Where the outage alert goes (plan A4): the owner's `dev_room_id`, parsed
/// from its storage form, else the Matrix room named "dev" as the registry
/// finds it. A configured value that does not parse is not replaced by the
/// named room: the owner chose something and it cannot be honoured.
async fn alert_target(
    dev_room_id: Option<&str>,
    registry: &ChannelRegistry,
) -> Option<ConversationAddress> {
    match dev_room_id {
        Some(raw) => match ConversationAddress::parse_storage(raw) {
            Ok(address) => Some(address),
            Err(err) => {
                warn!("Invalid dev_room_id {raw:?}: {err}");
                None
            }
        },
        None => {
            registry
                .find_conversation_by_name(Channel::Matrix, "dev")
                .await
        }
    }
}

/// Tells the owner's dev room that the AI provider is unreachable, once per outage.
pub async fn notify_dev_room_ai_error(
    settings: &SettingsService,
    outbound: Option<&Weak<ChannelRegistry>>,
    owner_id: &AccountId,
    error_msg: &str,
) {
    // Only send once per outage incident!
    if AI_ERROR_ALERTED.swap(true, Ordering::SeqCst) {
        debug!("AI outage error already alerted to DEV room; suppressing duplicate alert.");
        return;
    }
    deliver_alert(settings, outbound, owner_id, error_msg).await;
}

async fn deliver_alert(
    settings: &SettingsService,
    outbound: Option<&Weak<ChannelRegistry>>,
    owner_id: &AccountId,
    error_msg: &str,
) {
    let Some(registry) = outbound.and_then(Weak::upgrade) else {
        warn!("No channel registry available to send AI error alert to DEV room.");
        return;
    };

    let settings = settings.get_or_default(owner_id).await;
    let Some(target) = alert_target(settings.dev_room_id.as_deref(), &registry).await else {
        info!(
            "No DEV room configured or found for owner {}; skipping error alert.",
            owner_id
        );
        return;
    };

    let plain = format!(
        "⚠️ AI Assistant Error: Unable to reach the AI model provider. Please verify your AI settings or endpoint. ({})",
        error_msg
    );
    let html = format!(
        "⚠️ <b>AI Assistant Error</b>: Unable to reach the AI model provider.<br/><code>{}</code><br/><i>Please verify your AI settings or endpoint.</i>",
        html_escape(error_msg)
    );
    let message = OutgoingMessage {
        target: Target::Conversation(target.clone()),
        content: Content {
            plain,
            html: Some(html),
            ..Content::default()
        },
        reply_to: None,
        visibility: Visibility::Normal,
    };
    match registry.send(message).await {
        Ok(_) => info!(
            "Dispatched AI outage alert to DEV room {}",
            target.to_storage()
        ),
        Err(err) => error!(
            "Failed to send AI error alert to DEV room {}: {}",
            target.to_storage(),
            err
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::{Channel, ConversationAddress};
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::registry::ChannelRegistry;
    use std::sync::Arc;

    fn registry_with_dev_room(room: Option<&str>) -> ChannelRegistry {
        let mut adapter = FakeAdapter::connected(Channel::Matrix);
        if let Some(room) = room {
            adapter = adapter.with_conversation("dev", ConversationAddress::Matrix(room.into()));
        }
        ChannelRegistry::new(vec![Arc::new(adapter)]).unwrap()
    }

    /// A4: the configured `dev_room_id` wins, parsed from its storage form.
    #[tokio::test]
    async fn the_configured_dev_room_is_the_alert_target() {
        let registry = registry_with_dev_room(Some("!named:x"));
        assert_eq!(
            alert_target(Some("!set:x"), &registry).await,
            Some(ConversationAddress::Matrix("!set:x".into()))
        );
    }

    /// A4: with no setting, the Matrix room named "dev" is looked up through the registry.
    #[tokio::test]
    async fn without_a_setting_the_room_named_dev_is_found_through_the_registry() {
        let registry = registry_with_dev_room(Some("!named:x"));
        assert_eq!(
            alert_target(None, &registry).await,
            Some(ConversationAddress::Matrix("!named:x".into()))
        );
    }

    #[tokio::test]
    async fn no_setting_and_no_dev_room_means_no_alert() {
        assert_eq!(
            alert_target(None, &registry_with_dev_room(None)).await,
            None
        );
    }

    /// A malformed setting is not silently replaced by the named room: the
    /// owner chose something, and it cannot be honoured.
    #[tokio::test]
    async fn an_unparseable_setting_is_not_replaced_by_the_named_room() {
        let registry = registry_with_dev_room(Some("!named:x"));
        assert_eq!(alert_target(Some("not a room"), &registry).await, None);
    }

    #[tokio::test]
    async fn the_alert_goes_through_the_registry_with_escaped_detail() {
        let adapter = Arc::new(
            FakeAdapter::connected(Channel::Matrix)
                .with_conversation("dev", ConversationAddress::Matrix("!named:x".into())),
        );
        let registry = Arc::new(ChannelRegistry::new(vec![adapter.clone()]).unwrap());
        let weak = Arc::downgrade(&registry);
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let services = crate::application::AppServices::new(&pool);
        let owner = crate::infrastructure::postgres::repositories::accounts::core::create(
            &pool,
            "a",
            crate::domain::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();

        deliver_alert(&services.settings, Some(&weak), &owner, "boom <b>").await;

        let sent = adapter.sent();
        assert_eq!(sent.len(), 1);
        let html = sent[0].content.html.as_deref().unwrap();
        assert!(html.contains("<code>boom &lt;b&gt;</code>"), "{html}");
        assert!(matches!(
            &sent[0].target,
            crate::interface::channels::message::Target::Conversation(ConversationAddress::Matrix(r)) if r == "!named:x"
        ));
    }
}
