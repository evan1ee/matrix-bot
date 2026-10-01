//! In-memory adapter for registry, pipeline and delivery tests.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tokio_util::sync::CancellationToken;

use super::adapter::{ChannelAdapter, ChannelError, SentRef};
use super::message::OutgoingMessage;
use crate::domain::channels::Channel;

pub(crate) struct FakeAdapter {
    channel: Channel,
    connected: AtomicBool,
    sent: Mutex<Vec<OutgoingMessage>>,
}

impl FakeAdapter {
    pub(crate) fn connected(channel: Channel) -> Self {
        Self {
            channel,
            connected: AtomicBool::new(true),
            sent: Mutex::new(Vec::new()),
        }
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
}
