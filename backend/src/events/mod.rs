//! The events runtime: the SSE hub, the one `LISTEN outbox` per process,
//! hourly retention and the durable delivery dispatcher. SQL-free:
//! it reaches the outbox only through `OutboxService`.

mod dispatcher;
mod hub;
mod listener;
mod purge;

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::Notify;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::application::AppServices;

pub use dispatcher::DeliverySink;
pub use hub::{Hub, Notified, Subscription};

#[derive(Debug, Clone)]
pub struct EventsConfig {
    pub listener_retry: Duration,
    pub poll_interval: Duration,
    pub purge_interval: Duration,
    pub retain: Duration,
    pub replay_window: i64,
    pub replay_limit: i64,
    pub subscriber_capacity: usize,
    /// How long a claimed delivery stays invisible to other claimers.
    pub lease: Duration,
    /// Deliveries claimed per dispatcher round.
    pub batch: i64,
    /// Per-delivery send timeout; must stay below `lease` (C8) so a hung
    /// send can never overlap a re-claim of the same delivery.
    pub deliver_timeout: Duration,
}

impl EventsConfig {
    pub fn production() -> Self {
        Self {
            listener_retry: Duration::from_secs(30),
            poll_interval: Duration::from_secs(1),
            purge_interval: Duration::from_secs(3_600),
            retain: Duration::from_secs(30 * 86_400),
            replay_window: 100,
            replay_limit: 500,
            subscriber_capacity: 64,
            lease: Duration::from_secs(60),
            batch: 20,
            deliver_timeout: Duration::from_secs(30),
        }
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            listener_retry: Duration::from_millis(100),
            poll_interval: Duration::from_millis(100),
            lease: Duration::from_secs(1),
            deliver_timeout: Duration::from_millis(500),
            ..Self::production()
        }
    }
}

pub struct EventsRuntime {
    pool: PgPool,
    services: AppServices,
    hub: Hub,
    sink: Arc<dyn DeliverySink>,
    config: Arc<EventsConfig>,
}

impl EventsRuntime {
    pub fn new(
        pool: PgPool,
        services: AppServices,
        hub: Hub,
        sink: Arc<dyn DeliverySink>,
        config: EventsConfig,
    ) -> Self {
        Self {
            pool,
            services,
            hub,
            sink,
            config: Arc::new(config),
        }
    }

    /// Runs until `shutdown`; a task ending earlier is a bug, logged, and
    /// ends the runtime so `app::run` supervision stops the process.
    pub async fn run(self, shutdown: CancellationToken) {
        let wake = Arc::new(Notify::new());
        let mut tasks = JoinSet::new();
        tasks.spawn(listener::run(
            self.pool.clone(),
            self.hub.clone(),
            wake.clone(),
            self.config.clone(),
            shutdown.clone(),
        ));
        tasks.spawn(purge::run(
            self.services.outbox.clone(),
            self.config.clone(),
            shutdown.clone(),
        ));
        tasks.spawn(dispatcher::run(
            self.services.outbox.clone(),
            self.sink.clone(),
            wake.clone(),
            self.config.clone(),
            shutdown.clone(),
        ));
        while let Some(result) = tasks.join_next().await {
            if !shutdown.is_cancelled() {
                error!(?result, "An events runtime task ended before shutdown");
                shutdown.cancel();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C8: a hung send must time out before its lease expires, or a second
    /// claimer could deliver the same event concurrently.
    #[test]
    fn the_delivery_timeout_is_shorter_than_the_lease() {
        for config in [EventsConfig::production(), EventsConfig::for_tests()] {
            assert!(config.deliver_timeout < config.lease);
        }
        assert_eq!(EventsConfig::production().lease, Duration::from_secs(60));
        assert_eq!(
            EventsConfig::production().deliver_timeout,
            Duration::from_secs(30)
        );
    }
}
