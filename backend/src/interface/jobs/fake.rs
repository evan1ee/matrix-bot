//! Scripted `Outbound` for handler tests: records every message, never
//! touches a platform.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::Outbound;
use crate::domain::channels::Channel;
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::message::{MessageRef, OutgoingMessage, Target};

#[derive(Default)]
pub(crate) struct FakeOutbound {
    /// One flag for every channel: a test scripts "the platform is down".
    pub connected: AtomicBool,
    /// Popped per send; empty means `Ok` with the event id `$evt`.
    pub results: Mutex<VecDeque<Result<SentRef, ChannelError>>>,
    pub sent: Mutex<Vec<OutgoingMessage>>,
}

impl FakeOutbound {
    pub fn connected() -> Arc<Self> {
        let fake = Self::default();
        fake.connected.store(true, Ordering::SeqCst);
        Arc::new(fake)
    }

    pub fn disconnected() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn failing_once(reason: &str) -> Arc<Self> {
        let fake = Self::connected();
        fake.results
            .lock()
            .unwrap()
            .push_back(Err(ChannelError::Transient(reason.to_string())));
        fake
    }

    /// `(conversation in storage form, plain, html)` of every send to a conversation.
    pub fn room_sends(&self) -> Vec<(String, String, String)> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| match &m.target {
                Target::Conversation(address) => Some((
                    address.to_storage(),
                    m.content.plain.clone(),
                    m.content.html.clone().unwrap_or_default(),
                )),
                Target::Direct(_) => None,
            })
            .collect()
    }

    /// `(user id, plain, html)` of every direct send.
    pub fn dms(&self) -> Vec<(String, String, String)> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter_map(|m| match &m.target {
                Target::Direct(identity) => Some((
                    identity.user_id().as_str().to_string(),
                    m.content.plain.clone(),
                    m.content.html.clone().unwrap_or_default(),
                )),
                Target::Conversation(_) => None,
            })
            .collect()
    }

    pub fn messages(&self) -> Vec<OutgoingMessage> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl Outbound for FakeOutbound {
    fn is_connected(&self, _channel: Channel) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        if let Some(channel) = message.channel() {
            if !self.is_connected(channel) {
                return Err(ChannelError::NotConnected(channel));
            }
        }
        self.sent.lock().unwrap().push(message);
        self.results.lock().unwrap().pop_front().unwrap_or_else(|| {
            Ok(SentRef {
                message: Some(MessageRef("$evt".to_string())),
            })
        })
    }
}
