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
    app_config: crate::application::config::AppConfigService,
    config: Arc<EventsConfig>,
    shutdown: CancellationToken,
) {
    // Seeded once, on the first successful connect: later connects must
    // replay what the outage missed, not skip it.
    let mut needs_seed = true;

    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            result = listen(&pool, &hub, &wake, &app_config, &config, &mut needs_seed) => {
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
    app_config: &crate::application::config::AppConfigService,
    config: &EventsConfig,
    needs_seed: &mut bool,
) -> sqlx::Result<std::convert::Infallible> {
    // LISTEN first, then read the floor: an event committing in between is
    // both notified and at or below the seed, never lost.
    let mut listener = OutboxNotifications::connect(pool).await?;
    if *needs_seed {
        *needs_seed = !hub.seed().await;
    }
    wake.notify_waiters();
    hub.replay(config.replay_window, config.replay_limit).await;
    if let Err(err) = app_config.reload().await {
        warn!(error = %err, "Failed to reload runtime configuration snapshot on LISTEN connect");
    }

    let mut ttl_interval = tokio::time::interval(std::time::Duration::from_secs(60));
    ttl_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Consume the initial immediate tick so the periodic loop waits 60s.
    ttl_interval.tick().await;

    loop {
        // One `next()` future lives across TTL ticks. When a connection loss
        // is being handled, `next()` awaits the reconnect and only then
        // reports `Reconnected`; dropping it mid-way (a tick winning a
        // `select!` that re-creates it) makes sqlx reconnect silently and the
        // replay below would never run.
        let next = listener.next();
        tokio::pin!(next);
        let signal = loop {
            tokio::select! {
                _ = ttl_interval.tick() => {
                    if let Err(err) = app_config.reload().await {
                        warn!(error = %err, "Periodic runtime configuration snapshot reload failed");
                    }
                }
                signal = &mut next => break signal?,
            }
        };
        match signal {
            OutboxSignal::Event(id, account) => {
                hub.on_notification(id, account).await;
                wake.notify_waiters();
            }
            OutboxSignal::Reconnected => {
                warn!("Outbox LISTEN connection was lost and re-established; replaying");
                wake.notify_waiters();
                hub.replay(config.replay_window, config.replay_limit).await;
                if let Err(err) = app_config.reload().await {
                    warn!(error = %err, "Failed to reload runtime configuration snapshot after reconnect");
                }
            }
            OutboxSignal::Malformed => {}
        }
    }
}
