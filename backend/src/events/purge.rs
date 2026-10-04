//! Hourly retention (spec O6). Idempotent, so no leader lock.

use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::application::identity::LinkService;
use crate::application::outbox::OutboxService;
use crate::events::EventsConfig;

pub(crate) async fn run(
    outbox: OutboxService,
    link: LinkService,
    config: Arc<EventsConfig>,
    shutdown: CancellationToken,
) {
    let mut tick = tokio::time::interval(config.purge_interval);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tick.tick() => {}
        }
        match outbox.purge(config.retain).await {
            Ok(0) => {}
            Ok(n) => info!(deleted = n, "Purged finished outbox events"),
            Err(err) => warn!(error = %err, "Outbox purge failed; retrying next interval"),
        }
        match link.purge_expired(Duration::from_secs(86_400)).await {
            Ok(0) => {}
            Ok(n) => info!(deleted = n, "Purged expired link tokens"),
            Err(err) => warn!(error = %err, "Link token purge failed; retrying next interval"),
        }
    }
}
