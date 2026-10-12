//! The adapters configured in this process, keyed by channel.

use std::collections::HashMap;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::adapter::{ChannelAdapter, ChannelError, SentRef};
use super::message::OutgoingMessage;
use crate::domain::channels::{Channel, ConversationAddress};

pub struct ChannelRegistry {
    adapters: HashMap<Channel, Arc<dyn ChannelAdapter>>,
}

/// Refuses a list naming one channel twice. Shared with the composition root,
/// which checks the configured channels before it builds any adapter.
pub fn check_unique(channels: &[Channel]) -> anyhow::Result<()> {
    let mut seen = Vec::with_capacity(channels.len());
    for channel in channels {
        if seen.contains(channel) {
            anyhow::bail!("channel {channel} is configured twice");
        }
        seen.push(*channel);
    }
    Ok(())
}

impl ChannelRegistry {
    pub fn new(adapters: Vec<Arc<dyn ChannelAdapter>>) -> anyhow::Result<Self> {
        let channels: Vec<Channel> = adapters.iter().map(|a| a.channel()).collect();
        check_unique(&channels)?;
        Ok(Self::from_unique(adapters))
    }

    /// For `Arc::new_cyclic` in the composition root, whose closure cannot
    /// return an error. The caller must have run [`check_unique`] on these
    /// adapters' channels immediately before; were one repeated, the last
    /// adapter for that channel would silently win.
    pub fn from_unique(adapters: Vec<Arc<dyn ChannelAdapter>>) -> Self {
        Self {
            adapters: adapters.into_iter().map(|a| (a.channel(), a)).collect(),
        }
    }

    pub fn get(&self, channel: Channel) -> Option<&Arc<dyn ChannelAdapter>> {
        self.adapters.get(&channel)
    }

    pub fn is_connected(&self, channel: Channel) -> bool {
        self.get(channel).is_some_and(|a| a.is_connected())
    }

    /// Whether any configured adapter is connected: the outbox dispatcher's
    /// "worth claiming" test while it cannot tell a delivery's channel in advance.
    pub fn any_connected(&self) -> bool {
        self.adapters.values().any(|a| a.is_connected())
    }

    /// Routes by the target's channel. Web has none, so it is never delivered.
    pub async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        let channel = message.channel().ok_or(ChannelError::NotConfigured(None))?;
        let adapter = self
            .get(channel)
            .ok_or(ChannelError::NotConfigured(Some(channel)))?;
        if !adapter.is_connected() {
            return Err(ChannelError::NotConnected(channel));
        }
        adapter.send(message).await
    }

    pub async fn authorize_delivery(
        &self,
        owner: &crate::domain::accounts::AccountId,
        to: &ConversationAddress,
    ) -> Result<(), String> {
        let channel = to
            .channel()
            .ok_or_else(|| format!("Unsupported destination: {}", to.to_storage()))?;
        let adapter = self
            .get(channel)
            .ok_or_else(|| format!("{channel} is not configured"))?;
        adapter.authorize_delivery(owner, to).await
    }

    pub async fn find_conversation_by_name(
        &self,
        channel: Channel,
        name: &str,
    ) -> Option<ConversationAddress> {
        self.get(channel)?.find_conversation_by_name(name).await
    }

    /// Spawns every adapter's `run` under a child token and returns the join
    /// handles for `app::run` to supervise.
    pub fn spawn_all(
        &self,
        cancel: &CancellationToken,
    ) -> Vec<(Channel, tokio::task::JoinHandle<anyhow::Result<()>>)> {
        self.adapters
            .iter()
            .map(|(channel, adapter)| {
                let handle = tokio::spawn(Arc::clone(adapter).run(cancel.child_token()));
                (*channel, handle)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::{Channel, ConversationAddress};
    use crate::interface::channels::adapter::ChannelError;
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::message::{OutgoingMessage, Reply, Target};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    #[tokio::test]
    async fn send_routes_by_the_target_channel() {
        let m = Arc::new(FakeAdapter::connected(Channel::Matrix));
        let t = Arc::new(FakeAdapter::connected(Channel::Telegram));
        let reg = ChannelRegistry::new(vec![m.clone(), t.clone()]).unwrap();
        reg.send(OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Telegram(42)),
            Reply::html("<b>hi</b>"),
        ))
        .await
        .unwrap();
        assert_eq!(t.sent().len(), 1);
        assert!(m.sent().is_empty());
    }

    #[tokio::test]
    async fn an_unconfigured_channel_or_web_is_not_configured() {
        let reg =
            ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Matrix))]).unwrap();
        let to_tg = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Telegram(1)),
            Reply::html("x"),
        );
        assert!(matches!(
            reg.send(to_tg).await,
            Err(ChannelError::NotConfigured(Some(Channel::Telegram)))
        ));
        let to_web = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Web),
            Reply::html("x"),
        );
        assert!(matches!(
            reg.send(to_web).await,
            Err(ChannelError::NotConfigured(None))
        ));
    }

    #[tokio::test]
    async fn a_configured_but_disconnected_channel_is_not_connected() {
        let reg = ChannelRegistry::new(vec![Arc::new(FakeAdapter::disconnected(Channel::Matrix))])
            .unwrap();
        let msg = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Matrix("!r:s".into())),
            Reply::html("x"),
        );
        assert!(matches!(
            reg.send(msg).await,
            Err(ChannelError::NotConnected(Channel::Matrix))
        ));
        assert!(!reg.is_connected(Channel::Matrix));
    }

    #[test]
    fn check_unique_names_the_duplicate() {
        assert!(check_unique(&[Channel::Matrix, Channel::Telegram]).is_ok());
        let err = check_unique(&[Channel::Matrix, Channel::Matrix]).unwrap_err();
        assert!(err.to_string().contains("matrix"));
    }

    #[test]
    fn two_adapters_for_one_channel_are_refused() {
        let a = Arc::new(FakeAdapter::connected(Channel::Matrix));
        let b = Arc::new(FakeAdapter::connected(Channel::Matrix));
        assert!(ChannelRegistry::new(vec![a, b]).is_err());
    }

    #[tokio::test]
    async fn spawned_adapters_stop_when_the_token_is_cancelled() {
        let reg =
            ChannelRegistry::new(vec![Arc::new(FakeAdapter::connected(Channel::Matrix))]).unwrap();
        let cancel = CancellationToken::new();
        let handles = reg.spawn_all(&cancel);
        cancel.cancel();
        for (_, h) in handles {
            tokio::time::timeout(Duration::from_secs(1), h)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
        }
    }
}
