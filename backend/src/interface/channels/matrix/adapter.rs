//! The Matrix `ChannelAdapter`: the sync loop, the event handlers and the
//! outbound path behind the neutral contract.

use std::sync::{Arc, OnceLock};

use matrix_sdk::RoomState;
use tokio_util::sync::CancellationToken;

use crate::application::AppServices;
use crate::config::Config;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelAdapter, ChannelError, SentRef};
use crate::interface::channels::inbound::Inbound;
use crate::interface::channels::matrix::handles::BotHandles;
use crate::interface::channels::matrix::{outbound, rooms, sync_loop};
use crate::interface::channels::message::OutgoingMessage;

pub struct MatrixAdapter {
    pub(super) config: Config,
    pub(super) services: AppServices,
    pub(super) db: Database,
    /// Shared with the API: the client cell, the sync health and the pending merges.
    pub(super) handles: BotHandles,
    /// Set once by the composition root inside `Arc::new_cyclic`, because the
    /// pipeline holds a `Weak` of the registry that owns this adapter (A5).
    pub(super) inbound: OnceLock<Arc<Inbound>>,
}

impl MatrixAdapter {
    pub fn new(config: Config, services: AppServices, db: Database, handles: BotHandles) -> Self {
        Self {
            config,
            services,
            db,
            handles,
            inbound: OnceLock::new(),
        }
    }

    /// Called once from `app.rs`; a second call keeps the first pipeline.
    pub fn attach_inbound(&self, inbound: Arc<Inbound>) {
        if self.inbound.set(inbound).is_err() {
            tracing::warn!("The Matrix inbound pipeline was already attached; keeping the first");
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for MatrixAdapter {
    fn channel(&self) -> Channel {
        Channel::Matrix
    }

    fn is_connected(&self) -> bool {
        self.handles.matrix.current().is_some()
    }

    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()> {
        let inbound = self
            .inbound
            .get()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("the Matrix inbound pipeline was never attached"))?;
        sync_loop::run_bot_loop(&self, inbound, cancel).await;
        Ok(())
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        outbound::send(&self.handles.matrix, message).await
    }

    /// Today's room-authorization check (`MatrixRoomSink`, the webhook endpoints).
    async fn authorize_delivery(&self, owner: &AccountId, to: &ConversationAddress) -> bool {
        let ConversationAddress::Matrix(room_id) = to else {
            return false;
        };
        rooms::is_room_authorized_for_account(
            &self.services.rooms,
            self.handles.matrix.current().as_ref(),
            owner,
            room_id,
        )
        .await
    }

    /// The joined room whose name is `name` (the AI outage alert's fallback).
    async fn find_conversation_by_name(&self, name: &str) -> Option<ConversationAddress> {
        let client = self.handles.matrix.current()?;
        client
            .joined_rooms()
            .into_iter()
            .find(|room| {
                room.state() == RoomState::Joined
                    && room
                        .name()
                        .is_some_and(|n| n.trim().eq_ignore_ascii_case(name))
            })
            .map(|room| ConversationAddress::Matrix(room.room_id().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::domain::channels::{Channel, ConversationAddress};
    use crate::interface::channels::message::{OutgoingMessage, Reply, Target};

    async fn adapter() -> MatrixAdapter {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        MatrixAdapter::new(
            Config::for_test(),
            AppServices::new(&pool),
            Database { pool },
            BotHandles::default(),
        )
    }

    fn to_room(room: &str) -> OutgoingMessage {
        OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Matrix(room.into())),
            Reply::html("<b>hi</b>"),
        )
    }

    #[tokio::test]
    async fn without_a_client_it_is_disconnected_and_every_send_is_not_connected() {
        let a = adapter().await;
        assert_eq!(a.channel(), Channel::Matrix);
        assert!(!a.is_connected());
        assert!(matches!(
            a.send(to_room("!r:x")).await,
            Err(ChannelError::NotConnected(Channel::Matrix))
        ));
    }

    #[tokio::test]
    async fn a_target_on_another_channel_is_rejected() {
        let a = adapter().await;
        let msg = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Telegram(5)),
            Reply::html("x"),
        );
        assert!(matches!(a.send(msg).await, Err(ChannelError::Rejected(_))));
    }

    #[tokio::test]
    async fn a_malformed_room_id_is_rejected_not_retried() {
        let a = adapter().await;
        let client = matrix_sdk::Client::builder()
            .homeserver_url("http://localhost:1")
            .build()
            .await
            .unwrap();
        let _published = a.handles.matrix.publish(client);
        assert!(a.is_connected());
        assert!(matches!(
            a.send(to_room("not a room id")).await,
            Err(ChannelError::Rejected(_))
        ));
    }

    #[tokio::test]
    async fn without_a_client_nothing_is_authorized_or_found() {
        let a = adapter().await;
        let owner = crate::domain::accounts::AccountId::from_seed("owner");
        assert!(
            !a.authorize_delivery(&owner, &ConversationAddress::Matrix("!r:x".into()))
                .await
        );
        assert!(
            !a.authorize_delivery(&owner, &ConversationAddress::Telegram(1))
                .await
        );
        assert!(a.find_conversation_by_name("dev").await.is_none());
    }
}
