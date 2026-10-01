//! Delivers `matrix:<room>` outbox copies (webhook alerts) through the bot's
//! current Matrix client. Checks are the old webhook worker's; every error is
//! retryable (frozen). The delivery id is the Matrix transaction id, so a
//! resend after a crash within one session is deduplicated by the homeserver.

use async_trait::async_trait;
use matrix_sdk::ruma::{
    events::room::message::RoomMessageEventContent, OwnedTransactionId, RoomId,
};

use crate::application::outbox::ClaimedDelivery;
use crate::application::AppServices;
use crate::domain::events::{Destination, WebhookAlert};
use crate::events::DeliverySink;
use crate::interface::channels::matrix::live_client::MatrixHandle;

pub struct MatrixRoomSink {
    handle: MatrixHandle,
    services: AppServices,
}

impl MatrixRoomSink {
    pub fn new(handle: MatrixHandle, services: AppServices) -> Self {
        Self { handle, services }
    }
}

pub(crate) fn transaction_id(delivery: &ClaimedDelivery) -> OwnedTransactionId {
    OwnedTransactionId::from(format!("outbox-{}", delivery.id))
}

#[async_trait]
impl DeliverySink for MatrixRoomSink {
    fn is_ready(&self) -> bool {
        self.handle.current().is_some()
    }

    async fn deliver(&self, delivery: &ClaimedDelivery) -> Result<(), String> {
        let Some(Destination::MatrixRoom(room)) = Destination::parse(&delivery.destination) else {
            return Err(format!("Unsupported destination: {}", delivery.destination));
        };
        let alert: WebhookAlert = serde_json::from_value(delivery.event.json["payload"].clone())
            .map_err(|e| format!("Unreadable webhook alert: {e}"))?;
        let client = self.handle.current().ok_or("Matrix is not connected")?;
        let room_id = <&RoomId>::try_from(room.as_str())
            .map_err(|_| format!("Invalid Matrix room ID: {room}"))?;
        let joined = client
            .get_room(room_id)
            .ok_or_else(|| format!("Bot is not joined to room: {room}"))?;

        // The webhook owner must be authorized for the room at delivery time.
        let authorized = crate::interface::channels::matrix::rooms::is_room_authorized_for_account(
            &self.services.rooms,
            Some(&client),
            &delivery.event.account_id,
            &room,
        )
        .await;
        if !authorized {
            return Err(format!(
                "Delivery rejected: owner {} is not joined to room {room}",
                delivery.event.account_id
            ));
        }

        let content = RoomMessageEventContent::text_html(&alert.plain_body, &alert.html_body);
        joined
            .send(content)
            .with_transaction_id(transaction_id(delivery))
            .await
            .map_err(|err| format!("Matrix homeserver delivery failed: {err:?}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::outbox::StoredEvent;
    use crate::domain::accounts::AccountId;
    use crate::domain::events::{DeliveryId, EventId};

    #[test]
    fn the_matrix_transaction_id_is_derived_from_the_delivery_id() {
        let delivery = ClaimedDelivery {
            id: DeliveryId(42),
            destination: "matrix:!r:example.org".to_string(),
            attempts: 1,
            max_attempts: 10,
            event: StoredEvent {
                id: EventId(7),
                account_id: AccountId::from_seed("acct_txn"),
                event_type: "WebhookTriggered".to_string(),
                json: serde_json::Value::Null,
            },
        };
        assert_eq!(transaction_id(&delivery).as_str(), "outbox-42");
    }
}
