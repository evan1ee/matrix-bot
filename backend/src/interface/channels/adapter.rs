//! The contract every chat platform implements.

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::message::{MessageRef, OutgoingMessage};
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress};

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("{0} is not connected")]
    NotConnected(Channel),
    /// The channel has no adapter in this process, or the target is not a
    /// platform at all (`None`: the web UI, which nothing is delivered to).
    #[error("not configured: {0:?}")]
    NotConfigured(Option<Channel>),
    /// The platform refused the message; retrying the same payload will not help.
    #[error("rejected: {0}")]
    Rejected(String),
    /// A failure that may succeed on retry (network, rate limit).
    #[error("transient: {0}")]
    Transient(String),
}

/// What the platform reported back for a delivered message.
#[derive(Clone, Debug)]
pub struct SentRef {
    pub message: Option<MessageRef>,
}

#[async_trait::async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn channel(&self) -> Channel;
    fn is_connected(&self) -> bool;
    /// Runs the platform connection until `cancel` fires. Returning earlier is
    /// a failure the supervisor reports.
    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()>;
    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>;
    /// Outbox pre-check (webhook alerts); default allows.
    async fn authorize_delivery(&self, _owner: &AccountId, _to: &ConversationAddress) -> bool {
        true
    }
    /// A conversation found by display name (AI outage alert fallback, A4); default none.
    async fn find_conversation_by_name(&self, _name: &str) -> Option<ConversationAddress> {
        None
    }
}
