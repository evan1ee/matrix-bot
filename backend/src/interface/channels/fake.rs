//! In-memory adapter for registry, pipeline and delivery tests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use super::adapter::{ChannelAdapter, ChannelError, SentRef};
use super::message::OutgoingMessage;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress};

pub(crate) struct FakeAdapter {
    channel: Channel,
    connected: AtomicBool,
    sent: Mutex<Vec<OutgoingMessage>>,
    authorized: AtomicBool,
    named: HashMap<String, ConversationAddress>,
}

impl FakeAdapter {
    pub(crate) fn connected(channel: Channel) -> Self {
        Self {
            channel,
            connected: AtomicBool::new(true),
            sent: Mutex::new(Vec::new()),
            authorized: AtomicBool::new(true),
            named: HashMap::new(),
        }
    }

    /// A conversation `find_conversation_by_name` will find under `name`.
    pub(crate) fn with_conversation(mut self, name: &str, address: ConversationAddress) -> Self {
        self.named.insert(name.to_owned(), address);
        self
    }

    /// An adapter whose `authorize_delivery` refuses every owner.
    pub(crate) fn unauthorized(self) -> Self {
        self.authorized.store(false, Ordering::SeqCst);
        self
    }

    pub(crate) fn disconnected(channel: Channel) -> Self {
        let adapter = Self::connected(channel);
        adapter.connected.store(false, Ordering::SeqCst);
        adapter
    }

    pub(crate) fn sent(&self) -> Vec<OutgoingMessage> {
        self.sent.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for FakeAdapter {
    fn channel(&self) -> Channel {
        self.channel
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()> {
        cancel.cancelled().await;
        Ok(())
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        self.sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(message);
        Ok(SentRef { message: None })
    }

    async fn authorize_delivery(&self, _owner: &AccountId, _to: &ConversationAddress) -> bool {
        self.authorized.load(Ordering::SeqCst)
    }

    async fn find_conversation_by_name(&self, name: &str) -> Option<ConversationAddress> {
        self.named.get(name).cloned()
    }
}
