//! `Delivery` over the live Matrix client: what job handlers use to reach
//! rooms and DMs. Without a published client every send is `NotConnected`,
//! which handlers turn into a deferral rather than a failure.

use async_trait::async_trait;
use matrix_sdk::ruma::events::room::message::RoomMessageEventContent;
use matrix_sdk::ruma::RoomId;
use tracing::warn;

use crate::interface::channels::matrix::live_client::MatrixHandle;
use crate::interface::channels::matrix::outbound::send_dm_html;
use crate::interface::jobs::{Delivery, DeliveryError};

pub struct MatrixDelivery {
    handle: MatrixHandle,
}

impl MatrixDelivery {
    pub fn new(handle: MatrixHandle) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Delivery for MatrixDelivery {
    fn is_connected(&self) -> bool {
        self.handle.current().is_some()
    }

    async fn send_to_room(
        &self,
        room_id: &str,
        plain: &str,
        html: &str,
    ) -> Result<Option<String>, DeliveryError> {
        let client = self.handle.current().ok_or(DeliveryError::NotConnected)?;
        let Ok(room_id) = <&RoomId>::try_from(room_id) else {
            warn!(room_id, "Not a Matrix room id; nothing to deliver to");
            return Ok(None);
        };
        let content = RoomMessageEventContent::text_html(plain, html);
        if let Some(room) = client.get_room(room_id) {
            match room.send(content.clone()).await {
                Ok(resp) => return Ok(Some(resp.response.event_id.to_string())),
                Err(err) => warn!(%room_id, error = ?err, "Send failed; retrying via join"),
            }
        } else {
            warn!(%room_id, "No room handle in the store; joining");
        }
        let joined = client
            .join_room_by_id(room_id)
            .await
            .map_err(|e| DeliveryError::Failed(format!("join {room_id}: {e}")))?;
        let resp = joined
            .send(content)
            .await
            .map_err(|e| DeliveryError::Failed(format!("send to {room_id} after join: {e}")))?;
        Ok(Some(resp.response.event_id.to_string()))
    }

    async fn send_dm(&self, mxid: &str, plain: &str, html: &str) -> Result<(), DeliveryError> {
        let client = self.handle.current().ok_or(DeliveryError::NotConnected)?;
        // Logs its own failures (unchanged behaviour).
        send_dm_html(&client, mxid, plain, html).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn without_a_client_every_send_is_not_connected() {
        let d = MatrixDelivery::new(MatrixHandle::default());
        assert!(!d.is_connected());
        assert!(matches!(
            d.send_to_room("!r:x", "p", "<p>h</p>").await,
            Err(DeliveryError::NotConnected)
        ));
        assert!(matches!(
            d.send_dm("@u:x", "p", "<p>h</p>").await,
            Err(DeliveryError::NotConnected)
        ));
    }
}
