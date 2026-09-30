//! The events runtime: the SSE hub, the one `LISTEN outbox` per process,
//! hourly retention, and (Task 9) the durable delivery dispatcher. SQL-free:
//! it reaches the outbox only through `OutboxService`.

mod bridge;
mod broadcaster;
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

pub use broadcaster::EventBroadcaster;
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
        }
    }

    #[cfg(test)]
    pub fn for_tests() -> Self {
        Self {
            listener_retry: Duration::from_millis(100),
            poll_interval: Duration::from_millis(100),
            ..Self::production()
        }
    }
}

pub struct EventsRuntime {
    pool: PgPool,
    services: AppServices,
    hub: Hub,
    legacy: EventBroadcaster,
    config: Arc<EventsConfig>,
}

impl EventsRuntime {
    pub fn new(
        pool: PgPool,
        services: AppServices,
        hub: Hub,
        legacy: EventBroadcaster,
        config: EventsConfig,
    ) -> Self {
        Self {
            pool,
            services,
            hub,
            legacy,
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
        tasks.spawn(bridge::run(
            self.legacy.clone(),
            self.hub.clone(),
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
