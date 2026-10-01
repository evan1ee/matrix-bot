//! Delivers outbox copies (webhook alerts, link confirmations) through the channel registry. The
//! checks are the old Matrix sink's, now the adapter's `authorize_delivery`
//! (joined, and the owner authorized, at delivery time), and every error is retryable (frozen). The
//! delivery id travels as the message's idempotency key, which Matrix uses as
//! the transaction id, so a resend after a crash within one session is
//! deduplicated by the homeserver.

use std::sync::Arc;

use async_trait::async_trait;

use crate::application::outbox::ClaimedDelivery;
use crate::domain::channels::Channel;
use crate::domain::events::{Destination, WebhookAlert};
use crate::events::DeliverySink;
use crate::interface::channels::adapter::ChannelError;
use crate::interface::channels::message::{Content, OutgoingMessage, Reply, Target, Visibility};
use crate::interface::channels::registry::ChannelRegistry;
use crate::utils::html::html_escape;

pub struct ChannelSink {
    registry: Arc<ChannelRegistry>,
}

impl ChannelSink {
    pub fn new(registry: Arc<ChannelRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl DeliverySink for ChannelSink {
    /// Idle while no adapter is connected, so the dispatcher does not burn
    /// attempts. A delivery's channel is unknown before it is claimed, so with
    /// several channels one that is down still costs its deliveries an attempt
    /// (`deliver` fails them as not connected, and they back off).
    fn is_ready(&self) -> bool {
        self.registry.any_connected()
    }

    async fn deliver(&self, delivery: &ClaimedDelivery) -> Result<(), String> {
        let Some(Destination::Conversation(address)) = Destination::parse(&delivery.destination)
        else {
            return Err(format!("Unsupported destination: {}", delivery.destination));
        };
        let channel = address
            .channel()
            .ok_or_else(|| format!("Unsupported destination: {}", delivery.destination))?;
        let adapter = self
            .registry
            .get(channel)
            .ok_or_else(|| format!("{channel} is not configured"))?;
        let payload = &delivery.event.json["payload"];

        let content = match delivery.event.event_type.as_str() {
            "WebhookTriggered" => {
                let alert: WebhookAlert = serde_json::from_value(payload.clone())
                    .map_err(|e| format!("Unreadable webhook alert: {e}"))?;
                // The webhook owner chose the room, so they must be authorized
                // for it at delivery time.
                adapter
                    .authorize_delivery(&delivery.event.account_id, &address)
                    .await?;
                Content {
                    plain: alert.plain_body,
                    html: Some(alert.html_body),
                    ..Content::default()
                }
            }
            // No `authorize_delivery` (R82): the destination is not owner-chosen
            // but the direct conversation the linking identity itself wrote to,
            // recorded by the service when it minted the token.
            "IdentityLinked" => identity_linked_content(payload)?,
            other => return Err(format!("Unsupported event type: {other}")),
        };

        let mut message = OutgoingMessage::to(
            Target::Conversation(address),
            Reply {
                content,
                visibility: Visibility::Normal,
            },
        );
        message.idempotency_key = Some(format!("outbox-{}", delivery.id));
        message.join_if_needed = false;
        self.registry
            .send(message)
            .await
            .map(|_| ())
            // The adapter's own text, unprefixed, so `last_error` in the
            // webhook log reads as it always did.
            .map_err(|err| match err {
                ChannelError::Transient(reason) | ChannelError::Rejected(reason) => reason,
                other => other.to_string(),
            })
    }
}

/// "✓ <Channel> account linked. …", from the event's payload.
fn identity_linked_content(payload: &serde_json::Value) -> Result<Content, String> {
    let channel: Channel = serde_json::from_value(payload["channel"].clone())
        .map_err(|e| format!("Unreadable identity link: {e}"))?;
    let label = channel.label();
    let sentence = " account linked. Your workspace is now available here.";
    Ok(Content {
        plain: format!("✓ {label}{sentence}"),
        html: Some(format!("✓ {}{sentence}", html_escape(label))),
        ..Content::default()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::application::outbox::StoredEvent;
    use crate::domain::accounts::AccountId;
    use crate::domain::channels::{Channel, ConversationAddress};
    use crate::domain::events::{DeliveryId, EventId, SystemEvent, WebhookAlert};
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::message::Target;

    fn alert(owner: AccountId, room: &str) -> WebhookAlert {
        WebhookAlert {
            id: "whk_1".into(),
            owner_id: owner,
            room_id: room.into(),
            severity: "info".into(),
            title: None,
            html_body: "<b>deploy</b> done".into(),
            plain_body: "deploy done".into(),
        }
    }

    fn delivery(id: i64, destination: &str, alert: WebhookAlert) -> ClaimedDelivery {
        let owner = alert.owner_id;
        let event = SystemEvent::WebhookTriggered(alert);
        ClaimedDelivery {
            id: DeliveryId(id),
            destination: destination.to_string(),
            attempts: 1,
            max_attempts: 10,
            event: StoredEvent {
                id: EventId(7),
                account_id: owner,
                event_type: "WebhookTriggered".to_string(),
                json: serde_json::to_value(&event).unwrap(),
            },
        }
    }

    fn sink(adapter: &Arc<FakeAdapter>) -> ChannelSink {
        let registry = ChannelRegistry::new(vec![adapter.clone()]).unwrap();
        ChannelSink::new(Arc::new(registry))
    }

    #[tokio::test]
    async fn delivers_to_the_parsed_conversation_with_the_delivery_id_as_idempotency_key() {
        let adapter = Arc::new(FakeAdapter::connected(Channel::Matrix));
        let owner = AccountId::from_seed("acct_sink_ok");

        let out = sink(&adapter)
            .deliver(&delivery(
                42,
                "matrix:!r:example.org",
                alert(owner, "!r:example.org"),
            ))
            .await;

        assert_eq!(out, Ok(()));
        let sent = adapter.sent();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            &sent[0].target,
            Target::Conversation(ConversationAddress::Matrix(room)) if room == "!r:example.org"
        ));
        assert_eq!(sent[0].content.plain, "deploy done");
        assert_eq!(sent[0].content.html.as_deref(), Some("<b>deploy</b> done"));
        assert_eq!(sent[0].idempotency_key.as_deref(), Some("outbox-42"));
        assert!(!sent[0].join_if_needed, "an outbox send never joins a room");
    }

    #[tokio::test]
    async fn an_owner_not_authorized_for_the_room_is_rejected_and_nothing_is_sent() {
        let adapter = Arc::new(FakeAdapter::connected(Channel::Matrix).unauthorized());
        let owner = AccountId::from_seed("acct_sink_deny");

        let out = sink(&adapter)
            .deliver(&delivery(
                1,
                "matrix:!r:example.org",
                alert(owner, "!r:example.org"),
            ))
            .await;

        assert_eq!(
            out,
            Err(format!(
                "Delivery rejected: owner {owner} is not joined to room !r:example.org"
            ))
        );
        assert!(adapter.sent().is_empty());
    }

    #[tokio::test]
    async fn a_destination_nobody_serves_is_an_error() {
        let adapter = Arc::new(FakeAdapter::connected(Channel::Matrix));
        let owner = AccountId::from_seed("acct_sink_bad");

        for destination in ["slack:1", "web", "telegram:42"] {
            let out = sink(&adapter)
                .deliver(&delivery(1, destination, alert(owner, "!r:x")))
                .await;
            assert!(out.is_err(), "{destination}: {out:?}");
        }
        assert!(adapter.sent().is_empty());
    }

    fn linked_delivery(id: i64, destination: &str, channel: Channel) -> ClaimedDelivery {
        let owner = AccountId::from_seed("acct_sink_linked");
        let event = SystemEvent::IdentityLinked {
            owner_id: owner,
            channel,
            user_id: "42".to_string(),
            conversation: "telegram:42".to_string(),
        };
        ClaimedDelivery {
            id: DeliveryId(id),
            destination: destination.to_string(),
            attempts: 1,
            max_attempts: 10,
            event: StoredEvent {
                id: EventId(9),
                account_id: owner,
                event_type: "IdentityLinked".to_string(),
                json: serde_json::to_value(&event).unwrap(),
            },
        }
    }

    #[tokio::test]
    async fn an_identity_linked_confirmation_is_sent_to_the_dm_that_asked() {
        // `unauthorized()`: the confirmation is not owner-chosen, so the
        // webhook room check must not apply to it (R82).
        let adapter = Arc::new(FakeAdapter::connected(Channel::Telegram).unauthorized());

        let out = sink(&adapter)
            .deliver(&linked_delivery(5, "telegram:42", Channel::Telegram))
            .await;

        assert_eq!(out, Ok(()));
        let sent = adapter.sent();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            &sent[0].target,
            Target::Conversation(ConversationAddress::Telegram(42))
        ));
        assert_eq!(
            sent[0].content.plain,
            "✓ Telegram account linked. Your workspace is now available here."
        );
        assert_eq!(
            sent[0].content.html.as_deref(),
            Some("✓ Telegram account linked. Your workspace is now available here.")
        );
        assert_eq!(sent[0].idempotency_key.as_deref(), Some("outbox-5"));
        assert!(!sent[0].join_if_needed);
    }

    #[tokio::test]
    async fn an_unreadable_identity_linked_payload_is_an_error() {
        let adapter = Arc::new(FakeAdapter::connected(Channel::Telegram));
        let mut delivery = linked_delivery(6, "telegram:42", Channel::Telegram);
        delivery.event.json = serde_json::json!({"type": "IdentityLinked", "payload": {}});
        assert!(sink(&adapter).deliver(&delivery).await.is_err());
        assert!(adapter.sent().is_empty());
    }

    #[tokio::test]
    async fn an_event_type_the_sink_does_not_render_is_an_error() {
        let adapter = Arc::new(FakeAdapter::connected(Channel::Telegram));
        let mut delivery = linked_delivery(7, "telegram:42", Channel::Telegram);
        delivery.event.event_type = "TodoCreated".to_string();
        assert!(sink(&adapter).deliver(&delivery).await.is_err());
        assert!(adapter.sent().is_empty());
    }

    #[test]
    fn it_is_ready_only_while_an_adapter_is_connected() {
        let up = Arc::new(FakeAdapter::connected(Channel::Matrix));
        assert!(sink(&up).is_ready());
        let down = Arc::new(FakeAdapter::disconnected(Channel::Matrix));
        assert!(!sink(&down).is_ready());
    }
}
