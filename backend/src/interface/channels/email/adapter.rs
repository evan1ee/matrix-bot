//! Email `ChannelAdapter`: manages IMAP polling, inbound dispatch through
//! the shared pipeline, DMARC-enforced drop, deduplication, and SMTP delivery.

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::imap::ImapPoller;
use super::inbound::{parse_inbound_email, InboundError};
use super::outbound::EmailSender;
use super::receipts::ingest_receipts;
use crate::config::EmailConfig;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress};
use crate::interface::channels::adapter::{ChannelAdapter, ChannelError, SentRef};
use crate::interface::channels::inbound::{Inbound, InboundOutcome};
use crate::interface::channels::message::{OutgoingMessage, Target};

/// Sliding window size for inbound message deduplication (E13 / R130).
pub const DEDUPE_WINDOW: usize = 1024;

/// Bounded sliding window for Message-ID deduplication.
pub struct RecentMessages {
    capacity: usize,
    order: VecDeque<String>,
    seen: HashSet<String>,
}

impl RecentMessages {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            seen: HashSet::with_capacity(capacity),
        }
    }

    /// Returns `true` the first time `id` is seen, `false` on redelivery.
    pub fn first_time(&mut self, id: &str) -> bool {
        if self.capacity == 0 {
            return true;
        }
        if !self.seen.insert(id.to_owned()) {
            return false;
        }
        self.order.push_back(id.to_owned());
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

pub struct EmailAdapter {
    config: EmailConfig,
    allowed_senders: Option<Vec<String>>,
    sender: EmailSender,
    poller: ImapPoller,
    connected: Arc<AtomicBool>,
    dedupe: Arc<Mutex<RecentMessages>>,
    inbound: OnceLock<Arc<Inbound>>,
}

impl EmailAdapter {
    /// Creates a new `EmailAdapter`.
    pub fn new(config: EmailConfig, allowed_senders: Option<Vec<String>>) -> anyhow::Result<Self> {
        let sender = EmailSender::new(&config)?;
        let poller = ImapPoller::new(config.clone());
        Ok(Self {
            config,
            allowed_senders,
            sender,
            poller,
            connected: Arc::new(AtomicBool::new(false)),
            dedupe: Arc::new(Mutex::new(RecentMessages::new(DEDUPE_WINDOW))),
            inbound: OnceLock::new(),
        })
    }

    /// Attaches the shared inbound pipeline (called from composition root).
    pub fn attach_inbound(&self, inbound: Arc<Inbound>) {
        if self.inbound.set(inbound).is_err() {
            warn!("The Email inbound pipeline was already attached; keeping the first");
        }
    }

    /// Runs a single poll cycle against IMAP, processing unseen emails.
    pub async fn poll_once(&self, inbound: &Inbound) {
        let raw_messages = match self.poller.fetch_unseen_raw().await {
            Ok(msgs) => msgs,
            Err(e) => {
                warn!(error = %e, "IMAP fetch unseen failed");
                return;
            }
        };

        for raw in raw_messages {
            match parse_inbound_email(
                &raw,
                &self.config.authserv_id,
                self.allowed_senders.as_deref(),
            ) {
                Ok(parsed) => {
                    let msg_id = parsed.message.message_id.0.clone();
                    let is_first = self.dedupe.lock().unwrap().first_time(&msg_id);
                    if !is_first {
                        debug!(msg_id, "dropping duplicate email message");
                        continue;
                    }

                    // An email carrying attachment parts is a receipt submission; its
                    // body is only a caption and must never reach a module or the AI.
                    let reply = if parsed.has_attachments() {
                        ingest_receipts(inbound.services(), &inbound.db().pool, &parsed).await
                    } else {
                        // Progress is None (NoProgress / R128)
                        match inbound.handle(&parsed.message, parsed.text, None).await {
                            InboundOutcome::Replied(reply) => Some(reply),
                            InboundOutcome::Ignored => None,
                        }
                    };

                    if let Some(reply) = reply {
                        let outgoing = OutgoingMessage {
                            target: Target::Conversation(parsed.message.conversation.clone()),
                            content: reply.content,
                            reply_to: Some(parsed.message.message_id.clone()),
                            visibility: reply.visibility,
                            idempotency_key: None,
                            join_if_needed: true,
                        };
                        let reply_subject = parsed.subject.as_deref();
                        if let Err(e) = self.sender.send_message(outgoing, reply_subject).await {
                            error!(error = %e, "failed to send email reply via SMTP");
                        }
                    }
                }
                Err(InboundError::AutoReply) => {
                    debug!("dropped auto-reply email without response");
                }
                Err(InboundError::Dmarc(e)) => {
                    warn!(
                        error = %e,
                        "dropped email that failed DMARC authentication (Invariant 8)"
                    );
                }
                Err(InboundError::SenderNotAllowed(s)) => {
                    debug!(sender = %s, "dropped email from sender not in allowlist");
                }
                Err(e) => {
                    warn!(error = %e, "failed to parse inbound email");
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for EmailAdapter {
    fn channel(&self) -> Channel {
        Channel::Email
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()> {
        let inbound = self.inbound.get().cloned().ok_or_else(|| {
            anyhow::anyhow!("The Email adapter was started without an inbound pipeline")
        })?;

        self.connected.store(true, Ordering::Relaxed);
        info!("Email channel adapter started; polling IMAP");

        let interval = Duration::from_secs(self.config.poll_seconds);

        // Run an initial poll immediately
        self.poll_once(&inbound).await;

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    break;
                }
                _ = tokio::time::sleep(interval) => {
                    self.poll_once(&inbound).await;
                }
            }
        }

        self.connected.store(false, Ordering::Relaxed);
        info!("Email channel adapter stopped");
        Ok(())
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        self.sender.send_message(message, None).await
    }

    async fn authorize_delivery(
        &self,
        _owner: &AccountId,
        to: &ConversationAddress,
    ) -> Result<(), String> {
        match to {
            ConversationAddress::Email(_) => Ok(()),
            other => Err(format!("target {other:?} is not an email address")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::message::Reply;
    use crate::interface::channels::registry::ChannelRegistry;
    use url::Url;

    fn test_email_config() -> EmailConfig {
        EmailConfig {
            imap_url: Url::parse("imaps://imap.example.com").unwrap(),
            smtp_url: Url::parse("smtps://smtp.example.com").unwrap(),
            username: "user".into(),
            password: "password".into(),
            from: "bot@example.com".into(),
            authserv_id: "mx.example.com".into(),
            poll_seconds: 60,
            processed_folder: None,
        }
    }

    #[test]
    fn test_recent_messages_dedupe() {
        let mut dedupe = RecentMessages::new(3);
        assert!(dedupe.first_time("msg-1"));
        assert!(!dedupe.first_time("msg-1"));
        assert!(dedupe.first_time("msg-2"));
        assert!(dedupe.first_time("msg-3"));
        assert!(dedupe.first_time("msg-4")); // evicts msg-1
        assert!(!dedupe.first_time("msg-2"));
        assert!(dedupe.first_time("msg-1")); // forgotten
    }

    #[tokio::test]
    async fn a_fresh_adapter_is_disconnected_and_will_not_run_without_a_pipeline() {
        let adapter = Arc::new(EmailAdapter::new(test_email_config(), None).unwrap());
        assert_eq!(adapter.channel(), Channel::Email);
        assert!(!adapter.is_connected());
        let err = adapter.run(CancellationToken::new()).await.unwrap_err();
        assert!(err.to_string().contains("pipeline"), "{err}");
    }

    #[tokio::test]
    async fn the_registry_does_not_send_through_a_disconnected_adapter() {
        let adapter = Arc::new(EmailAdapter::new(test_email_config(), None).unwrap());
        let reg = ChannelRegistry::new(vec![adapter]).unwrap();
        let msg = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Email("user@example.com".into())),
            Reply::html("hello"),
        );
        assert!(matches!(
            reg.send(msg).await,
            Err(ChannelError::NotConnected(Channel::Email))
        ));
    }

    #[tokio::test]
    async fn authorize_delivery_permits_email_and_rejects_others() {
        let adapter = EmailAdapter::new(test_email_config(), None).unwrap();
        let owner = AccountId::from(uuid::Uuid::new_v4());
        assert!(adapter
            .authorize_delivery(
                &owner,
                &ConversationAddress::Email("alice@example.com".into())
            )
            .await
            .is_ok());
        assert!(adapter
            .authorize_delivery(&owner, &ConversationAddress::Matrix("!room:server".into()))
            .await
            .is_err());
    }
}
