//! Job handlers: the interface layer of the job runtime. Each handler parses
//! its payload, calls application services and delivers through `Outbound`.
//! No SQL, no business rules.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use tracing::{debug, warn};

use crate::application::AppServices;
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelError, SentRef};
use crate::interface::channels::message::{Content, OutgoingMessage, Reply, Target, Visibility};
use crate::interface::channels::registry::ChannelRegistry;
use crate::jobs::{JobError, JobHandler};

pub mod agent;
pub mod briefing;
pub mod budgets;
pub(crate) mod dm;
pub mod knowledge;
pub mod media;
pub mod portfolio;
pub mod reminders;

#[cfg(test)]
pub(crate) mod fake;

/// How long a job waits before retrying when its channel is disconnected.
pub const CHANNEL_RETRY: Duration = Duration::from_secs(30);

/// How handlers reach a platform; a seam so handler tests never touch one.
/// `ChannelRegistry` is the production implementation.
#[async_trait]
pub trait Outbound: Send + Sync {
    /// Whether `channel` is configured on this deployment.
    fn is_configured(&self, channel: Channel) -> bool {
        let _ = channel;
        true
    }
    fn is_connected(&self, channel: Channel) -> bool;
    /// Whether this deployment is ready to evaluate alerts: true when at least
    /// one configured channel is connected, or when no external channels are
    /// configured at all (Web-only deployment).
    fn is_ready_for_alerts(&self) -> bool {
        Channel::ALL.into_iter().any(|c| self.is_connected(c))
    }
    async fn authorize_delivery(
        &self,
        owner: &crate::domain::accounts::AccountId,
        to: &ConversationAddress,
    ) -> Result<(), String> {
        let _ = (owner, to);
        Ok(())
    }
    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError>;
}

#[async_trait]
impl Outbound for ChannelRegistry {
    fn is_configured(&self, channel: Channel) -> bool {
        self.get(channel).is_some()
    }

    fn is_connected(&self, channel: Channel) -> bool {
        ChannelRegistry::is_connected(self, channel)
    }

    fn is_ready_for_alerts(&self) -> bool {
        self.is_empty() || self.any_connected()
    }

    async fn authorize_delivery(
        &self,
        owner: &crate::domain::accounts::AccountId,
        to: &ConversationAddress,
    ) -> Result<(), String> {
        ChannelRegistry::authorize_delivery(self, owner, to).await
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        ChannelRegistry::send(self, message).await
    }
}

/// Every job handler.
pub fn handlers(
    services: &AppServices,
    pool: crate::db::DbPool,
    outbound: Arc<dyn Outbound>,
    ocr: crate::infrastructure::inference::ocr::OcrClient,
    ai_env_config: crate::config::AiEnvConfig,
) -> Vec<Arc<dyn JobHandler>> {
    let agent_engine = crate::llm::agent::ReActEngine::from_proxy(ai_env_config.proxy.as_deref());
    vec![
        Arc::new(reminders::ReminderFireHandler::new(
            services.clone(),
            outbound.clone(),
        )),
        Arc::new(briefing::BriefingSendHandler::new(
            services.clone(),
            outbound.clone(),
        )),
        Arc::new(portfolio::CheckAlertsHandler::new(
            services.clone(),
            outbound.clone(),
        )),
        Arc::new(portfolio::RefreshPricesHandler::new(services.clone())),
        Arc::new(budgets::BudgetAlertHandler::new(
            services.clone(),
            outbound.clone(),
        )),
        Arc::new(media::ProcessReceiptHandler::new(
            services.clone(),
            outbound.clone(),
            ocr,
        )),
        Arc::new(agent::AgentExecuteRunHandler::new(
            services.clone(),
            pool,
            outbound,
            agent_engine,
            ai_env_config,
        )),
        Arc::new(knowledge::KnowledgeEmbedHandler::new(services.clone())),
        Arc::new(knowledge::KnowledgeBackfillHandler::new(services.clone())),
    ]
}

/// Where a stored room string (`room_id` of a reminder, a default room) is
/// delivered to. `None` for a string that is no address and for `web`: nothing
/// is delivered outside a platform, and the job counts as delivered (the
/// behaviour for a non-room target has always been "nothing to do").
pub(crate) fn deliverable(room: &str) -> Option<(ConversationAddress, Channel)> {
    match ConversationAddress::parse_storage(room) {
        Ok(address) => match address.channel() {
            Some(channel) => Some((address, channel)),
            None => {
                debug!(room, "Web conversation; nothing to deliver");
                None
            }
        },
        Err(err) => {
            warn!(room, error = %err, "Not a deliverable conversation; treating as delivered");
            None
        }
    }
}

/// An HTML message with its plain-text fallback for `target`.
pub(crate) fn message(target: Target, plain: &str, html: &str) -> OutgoingMessage {
    OutgoingMessage::to(
        target,
        Reply {
            content: Content {
                plain: plain.to_owned(),
                html: Some(html.to_owned()),
                ..Content::default()
            },
            visibility: Visibility::Normal,
            is_ai: false,
            owner_id: None,
        },
    )
}

/// What a failed send means for a job.
pub(crate) enum SendFailure {
    /// The channel is down: wait for it.
    Defer,
    /// May succeed on retry.
    Retry(String),
    /// Will never succeed: do not retry.
    Permanent(String),
}

impl From<ChannelError> for SendFailure {
    fn from(err: ChannelError) -> Self {
        match err {
            ChannelError::NotConnected(_) => Self::Defer,
            ChannelError::Transient(reason) => Self::Retry(reason),
            ChannelError::Rejected(reason) => Self::Permanent(reason),
            // Callers filter Web and unconfigured channels out first; were one
            // to slip through, no retry helps.
            ChannelError::NotConfigured(Some(ch)) => {
                Self::Permanent(format!("{ch} is not configured"))
            }
            ChannelError::NotConfigured(None) => {
                Self::Permanent("the web UI has no delivery".to_owned())
            }
        }
    }
}

/// A payload that does not parse never will: fail the job now.
pub(crate) fn parse_payload<T: DeserializeOwned>(
    payload: serde_json::Value,
) -> Result<T, JobError> {
    serde_json::from_value(payload)
        .map_err(|e| JobError::Permanent(anyhow::anyhow!("malformed payload: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_malformed_payload_is_permanent() {
        let r: Result<crate::domain::jobs::ReminderFirePayload, _> =
            parse_payload(serde_json::json!({"x": 1}));
        assert!(matches!(r, Err(JobError::Permanent(_))));
    }
}
