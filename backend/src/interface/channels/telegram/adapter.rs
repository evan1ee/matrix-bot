//! The Telegram `ChannelAdapter`: start-up, the update queue and its workers,
//! webhook registration or long polling, and the outbound path behind the
//! neutral contract.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use axum::body::Bytes;
use teloxide_core::payloads::SetWebhookSetters;
use teloxide_core::prelude::*;
use teloxide_core::types::{AllowedUpdate, BotCommand};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use url::Url;

use super::commands::ADVERTISED;
use super::ingress::{RecentUpdates, TelegramIngress, QUEUE_CAPACITY};
use super::outbound::{self, bounded, redact, TelegramSendDeduper, CALL_TIMEOUT};
use super::polling::{backoff, is_fatal, poll};
use super::worker::{run_worker, WorkerCtx, TELEGRAM_WORKERS};
use crate::config::{TelegramConfig, TelegramMode};
use crate::domain::channels::Channel;
use crate::interface::channels::adapter::{ChannelAdapter, ChannelError, SentRef};
use crate::interface::channels::inbound::Inbound;
use crate::interface::channels::message::OutgoingMessage;

/// The longest wait between retries (1 s doubling).
const MAX_RETRY_DELAY: Duration = Duration::from_secs(300);
/// Every client request is bounded by this; the long poll adds its own bound.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(70);
/// How many recent `update_id`s are remembered (T11).
const DEDUPE_WINDOW: usize = 1024;
/// The path the webhook endpoint is mounted on (Task 6).
const WEBHOOK_PATH: &str = "/api/webhooks/telegram";

pub struct TelegramAdapter {
    config: TelegramConfig,
    bot: Bot,
    web_url: String,
    connected: Arc<AtomicBool>,
    ingress: TelegramIngress,
    rx: StdMutex<Option<mpsc::Receiver<Bytes>>>,
    /// Set once by the composition root inside `Arc::new_cyclic` (A5).
    inbound: OnceLock<Arc<Inbound>>,
    deduper: TelegramSendDeduper,
}

impl TelegramAdapter {
    /// Fails only when the HTTP client cannot be built.
    pub fn new(config: TelegramConfig, web_url: String) -> anyhow::Result<Self> {
        let client = teloxide_core::net::default_reqwest_settings()
            .timeout(CLIENT_TIMEOUT)
            .build()
            .map_err(|err| anyhow::anyhow!("could not build the Telegram HTTP client: {err}"))?;
        let bot = Bot::with_client(config.token.clone(), client);
        let secret = match &config.mode {
            TelegramMode::Webhook { secret } => Some(secret.as_str()),
            TelegramMode::Polling => None,
        };
        let (ingress, rx) = TelegramIngress::channel(QUEUE_CAPACITY, secret);
        Ok(Self {
            config,
            bot,
            web_url,
            connected: Arc::new(AtomicBool::new(false)),
            ingress,
            rx: StdMutex::new(Some(rx)),
            inbound: OnceLock::new(),
            deduper: TelegramSendDeduper::default(),
        })
    }

    /// Called once from `app.rs`; a second call keeps the first pipeline.
    pub fn attach_inbound(&self, inbound: Arc<Inbound>) {
        if self.inbound.set(inbound).is_err() {
            warn!("The Telegram inbound pipeline was already attached; keeping the first");
        }
    }

    /// The webhook endpoint's handle; `None` in polling mode.
    pub fn ingress(&self) -> Option<TelegramIngress> {
        matches!(self.config.mode, TelegramMode::Webhook { .. }).then(|| self.ingress.clone())
    }

    /// `getMe` until it answers; the username drives mention handling (T3). A
    /// refused token is fatal (a wrong token fails at boot, as R91).
    async fn identify(&self, cancel: &CancellationToken) -> anyhow::Result<Option<String>> {
        let mut attempt = 0;
        loop {
            // The raw error is inspected before classifying: a refused token
            // is fatal and must not be matched on a message string.
            match tokio::time::timeout(CALL_TIMEOUT, self.bot.get_me().send()).await {
                Ok(Ok(me)) => {
                    return me
                        .user
                        .username
                        .clone()
                        .map(Some)
                        .ok_or_else(|| anyhow::anyhow!("the Telegram bot has no username (getMe)"))
                }
                Ok(Err(err)) if is_fatal(&err) => {
                    return Err(anyhow::anyhow!("Telegram refused the bot token (getMe)"));
                }
                Ok(Err(err)) => warn!(error = %redact(&err.to_string()), "Telegram getMe failed"),
                Err(_) => warn!("Telegram getMe timed out"),
            }
            if !backoff(&mut attempt, cancel).await {
                return Ok(None);
            }
        }
    }

    /// Webhook mode: `setWebhook` with backoff until it succeeds.
    async fn register_webhook(
        &self,
        secret: &str,
        cancel: &CancellationToken,
    ) -> anyhow::Result<bool> {
        let url = webhook_url(&self.web_url)?;
        let mut attempt = 0;
        loop {
            let call = self
                .bot
                .set_webhook(url.clone())
                .secret_token(secret.to_owned())
                .allowed_updates([AllowedUpdate::Message, AllowedUpdate::CallbackQuery]);
            match bounded(call.send()).await {
                Ok(_) => return Ok(true),
                Err(err) => {
                    warn!(error = %redact(&err.to_string()), "Telegram setWebhook failed");
                    if !backoff(&mut attempt, cancel).await {
                        return Ok(false);
                    }
                }
            }
        }
    }
}

/// 1 s, 2 s, 4 s ... capped at five minutes.
pub(super) fn retry_delay(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(secs).min(MAX_RETRY_DELAY)
}

/// `{WEB_URL}/api/webhooks/telegram`.
pub(super) fn webhook_url(web_url: &str) -> anyhow::Result<Url> {
    Url::parse(&format!("{}{WEBHOOK_PATH}", web_url.trim_end_matches('/')))
        .map_err(|err| anyhow::anyhow!("WEB_URL is not usable as a webhook base: {err}"))
}

#[async_trait::async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn channel(&self) -> Channel {
        Channel::Telegram
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()> {
        let inbound =
            self.inbound.get().cloned().ok_or_else(|| {
                anyhow::anyhow!("the Telegram inbound pipeline was never attached")
            })?;
        let rx = self
            .rx
            .lock()
            .map_err(|_| anyhow::anyhow!("the Telegram queue lock is poisoned"))?
            .take()
            .ok_or_else(|| anyhow::anyhow!("the Telegram adapter is already running"))?;
        let Some(username) = self.identify(&cancel).await? else {
            return Ok(());
        };
        let commands = ADVERTISED
            .iter()
            .map(|(name, description)| BotCommand::new(*name, *description));
        if let Err(err) = bounded(self.bot.set_my_commands(commands).send()).await {
            warn!(error = %err, "Could not advertise the Telegram commands");
        }

        let rx = Arc::new(Mutex::new(rx));
        let ctx = WorkerCtx {
            inbound,
            bot: self.bot.clone(),
            username: Arc::from(username),
            recent: Arc::new(StdMutex::new(RecentUpdates::new(DEDUPE_WINDOW))),
        };
        let workers_cancel = cancel.child_token();
        let mut workers = JoinSet::new();
        for _ in 0..TELEGRAM_WORKERS {
            workers.spawn(run_worker(
                Arc::clone(&rx),
                ctx.clone(),
                workers_cancel.clone(),
            ));
        }

        let outcome = match &self.config.mode {
            TelegramMode::Webhook { secret } => {
                match self.register_webhook(secret, &cancel).await {
                    Ok(true) => {
                        self.connected.store(true, Ordering::Relaxed);
                        info!("Telegram webhook registered");
                        cancel.cancelled().await;
                        Ok(())
                    }
                    Ok(false) => Ok(()),
                    Err(err) => Err(err),
                }
            }
            TelegramMode::Polling => poll(&self.bot, &self.ingress, &self.connected, &cancel).await,
        };

        self.connected.store(false, Ordering::Relaxed);
        workers_cancel.cancel();
        while workers.join_next().await.is_some() {}
        outcome
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        outbound::send(&self.bot, &self.deduper, message).await
    }
    // `authorize_delivery` and `find_conversation_by_name` keep their defaults:
    // the outbox sends Telegram only `IdentityLinked`, whose destination is not
    // owner-chosen, and the AI outage alert targets Matrix.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TelegramMode;
    use crate::domain::channels::ConversationAddress;
    use crate::interface::channels::message::{Reply, Target};
    use crate::interface::channels::registry::ChannelRegistry;

    fn adapter(mode: TelegramMode) -> TelegramAdapter {
        TelegramAdapter::new(
            TelegramConfig {
                token: "123:not-a-token".into(),
                mode,
            },
            "https://bot.example".into(),
        )
        .unwrap()
    }

    #[test]
    fn retries_back_off_from_one_second_up_to_five_minutes() {
        assert_eq!(retry_delay(0), Duration::from_secs(1));
        assert_eq!(retry_delay(1), Duration::from_secs(2));
        assert_eq!(retry_delay(8), Duration::from_secs(256));
        for attempt in [9, 10, 30, 63, 64, u32::MAX] {
            assert_eq!(retry_delay(attempt), Duration::from_secs(300));
        }
    }

    #[test]
    fn the_webhook_url_is_the_web_url_plus_the_endpoint() {
        for base in ["https://bot.example", "https://bot.example/"] {
            assert_eq!(
                webhook_url(base).unwrap().as_str(),
                "https://bot.example/api/webhooks/telegram"
            );
        }
        assert!(webhook_url("not a url").is_err());
    }

    #[tokio::test]
    async fn a_fresh_adapter_is_disconnected_and_will_not_run_without_a_pipeline() {
        let a = Arc::new(adapter(TelegramMode::Polling));
        assert_eq!(a.channel(), Channel::Telegram);
        assert!(!a.is_connected());
        let err = a.run(CancellationToken::new()).await.unwrap_err();
        assert!(err.to_string().contains("pipeline"), "{err}");
    }

    #[test]
    fn only_webhook_mode_exposes_an_ingress() {
        assert!(adapter(TelegramMode::Polling).ingress().is_none());
        let hook = adapter(TelegramMode::Webhook {
            secret: "abc".into(),
        });
        let ingress = hook.ingress().unwrap();
        assert!(ingress.secret_matches(Some(b"abc")));
    }

    #[tokio::test]
    async fn the_registry_does_not_send_through_a_disconnected_adapter() {
        let reg = ChannelRegistry::new(vec![Arc::new(adapter(TelegramMode::Polling))]).unwrap();
        let msg = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Telegram(1)),
            Reply::html("x"),
        );
        assert!(matches!(
            reg.send(msg).await,
            Err(ChannelError::NotConnected(Channel::Telegram))
        ));
    }
}
