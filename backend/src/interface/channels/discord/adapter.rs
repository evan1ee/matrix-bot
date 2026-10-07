//! The Discord `ChannelAdapter`: the gateway client, the bounded work queue
//! and its workers, and the outbound path behind the neutral contract.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serenity::all::{GatewayError, GatewayIntents, Http};
use serenity::Client;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use super::gateway::{Gateway, QUEUE_CAPACITY};
use super::outbound;
use super::worker::{run_worker, Work, DISCORD_WORKERS};
use crate::config::DiscordConfig;
use crate::domain::channels::Channel;
use crate::interface::channels::adapter::{ChannelAdapter, ChannelError, SentRef};
use crate::interface::channels::inbound::Inbound;
use crate::interface::channels::message::OutgoingMessage;

/// The longest wait between reconnects (B7).
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(300);
/// A session that stayed up this long resets the backoff.
const STABLE_SESSION: Duration = Duration::from_secs(60);

pub struct DiscordAdapter {
    config: DiscordConfig,
    /// For `send` and the workers; the gateway client builds its own.
    http: Arc<Http>,
    connected: Arc<AtomicBool>,
    /// Set once by the composition root inside `Arc::new_cyclic` (A5).
    inbound: OnceLock<Arc<Inbound>>,
}

impl DiscordAdapter {
    pub fn new(config: DiscordConfig) -> Self {
        let http = Arc::new(Http::new(&config.token));
        Self {
            config,
            http,
            connected: Arc::new(AtomicBool::new(false)),
            inbound: OnceLock::new(),
        }
    }

    /// Called once from `app.rs`; a second call keeps the first pipeline.
    pub fn attach_inbound(&self, inbound: Arc<Inbound>) {
        if self.inbound.set(inbound).is_err() {
            warn!("The Discord inbound pipeline was already attached; keeping the first");
        }
    }

    /// One gateway session: returns when Discord or `cancel` ends it.
    async fn session(
        &self,
        queue: mpsc::Sender<Work>,
        cancel: &CancellationToken,
    ) -> Result<(), Box<serenity::Error>> {
        let handler = Gateway {
            queue,
            connected: Arc::clone(&self.connected),
            guild_id: self.config.guild_id,
        };
        let intents = GatewayIntents::GUILDS | GatewayIntents::DIRECT_MESSAGES;
        let mut client = Client::builder(&self.config.token, intents)
            .event_handler(handler)
            .await?;
        let shards = Arc::clone(&client.shard_manager);
        let result = tokio::select! {
            _ = cancel.cancelled() => {
                shards.shutdown_all().await;
                Ok(())
            }
            result = client.start() => result.map_err(Box::new),
        };
        self.connected.store(false, Ordering::Relaxed);
        result
    }
}

/// B7: 1 s, 2 s, 4 s … capped at five minutes.
pub(super) fn reconnect_delay(attempt: u32) -> Duration {
    let secs = 1u64.checked_shl(attempt).unwrap_or(u64::MAX);
    Duration::from_secs(secs).min(MAX_RECONNECT_DELAY)
}

/// B7: the gateway errors that retrying cannot fix.
pub(super) fn is_fatal(err: &serenity::Error) -> bool {
    matches!(
        err,
        serenity::Error::Gateway(
            GatewayError::InvalidAuthentication
                | GatewayError::InvalidGatewayIntents
                | GatewayError::DisallowedGatewayIntents
        )
    )
}

#[async_trait::async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn channel(&self) -> Channel {
        Channel::Discord
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    async fn run(self: Arc<Self>, cancel: CancellationToken) -> anyhow::Result<()> {
        let inbound =
            self.inbound.get().cloned().ok_or_else(|| {
                anyhow::anyhow!("the Discord inbound pipeline was never attached")
            })?;
        let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
        let rx = Arc::new(Mutex::new(rx));
        let workers_cancel = cancel.child_token();
        let mut workers = JoinSet::new();
        for _ in 0..DISCORD_WORKERS {
            workers.spawn(run_worker(
                Arc::clone(&rx),
                Arc::clone(&inbound),
                Arc::clone(&self.http),
                workers_cancel.clone(),
            ));
        }

        let mut attempt = 0;
        let outcome = loop {
            let started = Instant::now();
            let result = self.session(tx.clone(), &cancel).await;
            if cancel.is_cancelled() {
                break Ok(());
            }
            match result {
                Err(err) if is_fatal(&err) => {
                    error!(error = %err, "Discord refused the bot; check DISCORD_TOKEN and the intents");
                    break Err(anyhow::anyhow!("Discord gateway: {err}"));
                }
                Err(err) => warn!(error = %err, "Discord gateway session ended"),
                Ok(()) => warn!("Discord gateway session ended without an error"),
            }
            if started.elapsed() >= STABLE_SESSION {
                attempt = 0;
            }
            let delay = reconnect_delay(attempt);
            attempt = attempt.saturating_add(1);
            info!(delay_secs = delay.as_secs(), "Reconnecting to Discord");
            tokio::select! {
                _ = cancel.cancelled() => break Ok(()),
                _ = tokio::time::sleep(delay) => {}
            }
        };

        workers_cancel.cancel();
        drop(tx);
        while workers.join_next().await.is_some() {}
        outcome
    }

    async fn send(&self, message: OutgoingMessage) -> Result<SentRef, ChannelError> {
        outbound::send(&self.http, message).await
    }
    // `authorize_delivery` and `find_conversation_by_name` keep their defaults:
    // the outbox sends Discord only `IdentityLinked`, whose destination is not
    // owner-chosen (R82), and the AI outage alert targets Matrix.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::channels::ConversationAddress;
    use crate::interface::channels::message::{Reply, Target};
    use crate::interface::channels::registry::ChannelRegistry;

    fn adapter() -> DiscordAdapter {
        DiscordAdapter::new(DiscordConfig {
            token: "not-a-token".into(),
            guild_id: None,
        })
    }

    #[test]
    fn reconnects_back_off_exponentially_up_to_five_minutes() {
        assert_eq!(reconnect_delay(0), Duration::from_secs(1));
        assert_eq!(reconnect_delay(1), Duration::from_secs(2));
        assert_eq!(reconnect_delay(8), Duration::from_secs(256));
        assert_eq!(reconnect_delay(9), MAX_RECONNECT_DELAY);
        assert_eq!(reconnect_delay(30), MAX_RECONNECT_DELAY);
        assert_eq!(reconnect_delay(u32::MAX), MAX_RECONNECT_DELAY);
    }

    #[test]
    fn only_authentication_and_intent_errors_are_fatal() {
        for fatal in [
            GatewayError::InvalidAuthentication,
            GatewayError::InvalidGatewayIntents,
            GatewayError::DisallowedGatewayIntents,
        ] {
            assert!(is_fatal(&serenity::Error::Gateway(fatal)));
        }
        assert!(!is_fatal(&serenity::Error::Gateway(GatewayError::Closed(
            None
        ))));
        assert!(!is_fatal(&serenity::Error::Other("network")));
    }

    #[tokio::test]
    async fn a_fresh_adapter_is_disconnected_and_will_not_run_without_a_pipeline() {
        let a = Arc::new(adapter());
        assert_eq!(a.channel(), Channel::Discord);
        assert!(!a.is_connected());
        let err = a.run(CancellationToken::new()).await.unwrap_err();
        assert!(err.to_string().contains("pipeline"), "{err}");
    }

    #[tokio::test]
    async fn the_registry_does_not_send_through_a_disconnected_adapter() {
        let reg = ChannelRegistry::new(vec![Arc::new(adapter())]).unwrap();
        let msg = OutgoingMessage::to(
            Target::Conversation(ConversationAddress::Discord(1)),
            Reply::html("x"),
        );
        assert!(matches!(
            reg.send(msg).await,
            Err(ChannelError::NotConnected(Channel::Discord))
        ));
    }
}
