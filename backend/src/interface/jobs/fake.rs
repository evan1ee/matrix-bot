//! Scripted `Outbound` for handler tests: records every message, never
//! touches a platform.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::Outbound;
use crate::domain::channels::Channel;
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::message::{MessageRef, OutgoingMessage, Target};

#[derive(Default)]
pub(crate) struct FakeOutbound {
    /// Default flag for channels not listed in `channel_states`.
    pub connected: AtomicBool,
    /// Optional per-channel connection state (`Some` overrides `connected`).
    pub channel_states: Mutex<HashMap<Channel, bool>>,
    /// Optional restricted set of configured channels (`None` means all).
    pub configured: Mutex<Option<HashSet<Channel>>>,
    /// Optional delivery authorization error returned by `authorize_delivery`.
    pub auth_error: Mutex<Option<String>>,
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

    pub fn with_configured(channels: impl IntoIterator<Item = Channel>) -> Arc<Self> {
        let fake = Self::default();
        fake.connected.store(true, Ordering::SeqCst);
        *fake.configured.lock().unwrap() = Some(channels.into_iter().collect());
        Arc::new(fake)
    }

    pub fn with_states(states: impl IntoIterator<Item = (Channel, bool)>) -> Arc<Self> {
        let fake = Self::default();
        let map: HashMap<Channel, bool> = states.into_iter().collect();
        *fake.configured.lock().unwrap() = Some(map.keys().copied().collect());
        *fake.channel_states.lock().unwrap() = map;
        Arc::new(fake)
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
    fn is_configured(&self, channel: Channel) -> bool {
        self.configured
            .lock()
            .unwrap()
            .as_ref()
            .is_none_or(|set| set.contains(&channel))
    }

    fn is_connected(&self, channel: Channel) -> bool {
        if !self.is_configured(channel) {
            return false;
        }
        self.channel_states
            .lock()
            .unwrap()
            .get(&channel)
            .copied()
            .unwrap_or_else(|| self.connected.load(Ordering::SeqCst))
    }

    fn is_ready_for_alerts(&self) -> bool {
        let configured = self.configured.lock().unwrap().clone();
        match configured {
            Some(set) if set.is_empty() => true,
            Some(set) => set.into_iter().any(|c| self.is_connected(c)),
            None => Channel::ALL.into_iter().any(|c| self.is_connected(c)),
        }
    }

    async fn authorize_delivery(
        &self,
        _owner: &crate::domain::accounts::AccountId,
        _to: &crate::domain::channels::ConversationAddress,
    ) -> Result<(), String> {
        if let Some(reason) = self.auth_error.lock().unwrap().clone() {
            return Err(reason);
        }
        Ok(())
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        if let Some(channel) = message.channel() {
            if !self.is_configured(channel) {
                return Err(ChannelError::NotConfigured(Some(channel)));
            }
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
