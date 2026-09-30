//! One `LISTEN outbox` per process. Each committed event goes to the SSE
//! hub; the dispatcher is woken. On every (re)connect the hub replays and
//! the dispatcher is woken once, because a NOTIFY sent while the listener
//! was down is gone.

use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::events::hub::Hub;
use crate::events::EventsConfig;
use crate::infrastructure::postgres::outbox_notifications::{OutboxNotifications, OutboxSignal};

pub(crate) async fn run(
    pool: PgPool,
    hub: Hub,
    wake: Arc<Notify>,
    config: Arc<EventsConfig>,
    shutdown: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            result = listen(&pool, &hub, &wake, &config) => {
                let Err(err) = result;
                warn!(error = %err, "Outbox LISTEN failed; SSE pauses, deliveries fall back to polling");
            }
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(config.listener_retry) => {}
        }
    }
}

async fn listen(
    pool: &PgPool,
    hub: &Hub,
    wake: &Notify,
    config: &EventsConfig,
) -> sqlx::Result<std::convert::Infallible> {
    let mut listener = OutboxNotifications::connect(pool).await?;
    wake.notify_waiters();
    hub.replay(config.replay_window, config.replay_limit).await;
    loop {
        match listener.next().await? {
            OutboxSignal::Event(id, account) => {
                hub.on_notification(id, account).await;
                wake.notify_waiters();
            }
            OutboxSignal::Reconnected => {
                warn!("Outbox LISTEN connection was lost and re-established; replaying");
                wake.notify_waiters();
                hub.replay(config.replay_window, config.replay_limit).await;
            }
            OutboxSignal::Malformed => {}
        }
    }
}
