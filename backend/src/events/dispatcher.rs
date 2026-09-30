//! Delivers durable outbox copies (spec §5.4). One task per process;
//! `SKIP LOCKED` keeps several processes apart and the claim lease makes a
//! crashed claim reappear. Idles while the sink is not ready, so a Matrix
//! outage burns no attempts.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use crate::application::outbox::{ClaimedDelivery, OutboxService};
use crate::domain::events::DeliveryStep;
use crate::events::EventsConfig;

#[async_trait]
pub trait DeliverySink: Send + Sync {
    /// False while the channel is down.
    fn is_ready(&self) -> bool;
    async fn deliver(&self, delivery: &ClaimedDelivery) -> Result<(), String>;
}

pub(crate) async fn run(
    outbox: OutboxService,
    sink: Arc<dyn DeliverySink>,
    wake: Arc<Notify>,
    config: Arc<EventsConfig>,
    shutdown: CancellationToken,
) {
    loop {
        // Register before draining so a NOTIFY during the drain is not lost.
        let notified = wake.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        loop {
            if shutdown.is_cancelled() {
                return;
            }
            let handled = dispatch_once(&outbox, sink.as_ref(), &config).await;
            if handled < config.batch as usize {
                break;
            }
        }
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = &mut notified => {}
            _ = tokio::time::sleep(config.poll_interval) => {}
        }
    }
}

/// Claims and delivers one batch; returns how many were claimed.
pub(crate) async fn dispatch_once(
    outbox: &OutboxService,
    sink: &dyn DeliverySink,
    config: &EventsConfig,
) -> usize {
    if !sink.is_ready() {
        return 0;
    }
    let claimed = match outbox.claim_due(config.batch, config.lease).await {
        Ok(c) => c,
        Err(err) => {
            warn!(error = %err, "Could not claim outbox deliveries");
            return 0;
        }
    };
    for delivery in &claimed {
        let outcome =
            match tokio::time::timeout(config.deliver_timeout, sink.deliver(delivery)).await {
                Ok(result) => result,
                Err(_) => Err(format!(
                    "delivery timed out after {:?}",
                    config.deliver_timeout
                )),
            };
        match outcome {
            Ok(()) => {
                info!(delivery_id = delivery.id.0, destination = %delivery.destination, "Delivered outbox event");
                if let Err(err) = outbox.record_delivered(delivery).await {
                    warn!(error = %err, delivery_id = delivery.id.0, "Delivered but could not record it; the lease will retry it");
                }
            }
            Err(msg) => match outbox.record_failure(delivery, &msg).await {
                Ok(DeliveryStep::Fail) => {
                    warn!(delivery_id = delivery.id.0, attempts = delivery.attempts, error = %msg, "Outbox delivery failed permanently")
                }
                Ok(DeliveryStep::RetryAfterAttempts(_)) => {
                    warn!(delivery_id = delivery.id.0, attempts = delivery.attempts, error = %msg, "Outbox delivery failed; will retry")
                }
                Err(err) => {
                    warn!(error = %err, delivery_id = delivery.id.0, "Could not record a failed delivery; the lease will retry it")
                }
            },
        }
    }
    claimed.len()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::domain::events::DeliveryId;

    struct FakeSink {
        ready: AtomicBool,
        fail: AtomicBool,
        calls: AtomicUsize,
        seen: Mutex<Vec<DeliveryId>>,
    }

    #[async_trait::async_trait]
    impl DeliverySink for FakeSink {
        fn is_ready(&self) -> bool {
            self.ready.load(Ordering::SeqCst)
        }
        async fn deliver(&self, d: &ClaimedDelivery) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(d.id);
            if self.fail.load(Ordering::SeqCst) {
                Err("down".into())
            } else {
                Ok(())
            }
        }
    }

    fn sink(ready: bool, fail: bool) -> Arc<FakeSink> {
        Arc::new(FakeSink {
            ready: AtomicBool::new(ready),
            fail: AtomicBool::new(fail),
            calls: AtomicUsize::new(0),
            seen: Mutex::new(Vec::new()),
        })
    }

    async fn services_with_alert(seed: &str) -> (AppServices, AccountId, DeliveryId) {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let (webhook, _token) = services
            .webhooks
            .create(&owner, "ci", "!room:example.org")
            .await
            .unwrap();
        let id = services
            .webhooks
            .enqueue_alert(
                &webhook,
                "!room:example.org",
                None,
                "<b>h</b>",
                "h",
                crate::domain::webhooks::WebhookSeverity::Info,
            )
            .await
            .unwrap();
        (services, owner, id)
    }

    #[tokio::test]
    async fn a_successful_delivery_is_marked_delivered_and_counted() {
        let (services, owner, id) = services_with_alert("acct_disp_ok").await;
        let s = sink(true, false);
        let n = dispatch_once(&services.outbox, s.as_ref(), &EventsConfig::for_tests()).await;
        assert_eq!(n, 1);
        assert_eq!(*s.seen.lock().unwrap(), vec![id]);
        let hooks = services.webhooks.list(&owner).await.unwrap();
        assert_eq!(hooks[0].total_delivered_count, 1);
        assert_eq!(
            dispatch_once(&services.outbox, s.as_ref(), &EventsConfig::for_tests()).await,
            0
        );
    }

    #[tokio::test]
    async fn a_failed_delivery_is_retried_later_and_counted() {
        let (services, owner, _id) = services_with_alert("acct_disp_fail").await;
        let s = sink(true, true);
        assert_eq!(
            dispatch_once(&services.outbox, s.as_ref(), &EventsConfig::for_tests()).await,
            1
        );
        // retry_delay(0) = 1 s: not due immediately.
        assert_eq!(
            dispatch_once(&services.outbox, s.as_ref(), &EventsConfig::for_tests()).await,
            0
        );
        let hooks = services.webhooks.list(&owner).await.unwrap();
        assert_eq!(hooks[0].failure_count, 1, "C1: every failed attempt counts");
        let (logs, _) = services
            .webhooks
            .list_logs(&owner, &hooks[0].id, 10, 0)
            .await
            .unwrap();
        assert_eq!(logs[0].status, "pending");
        assert_eq!(logs[0].attempts, 1);
        assert_eq!(logs[0].last_error.as_deref(), Some("down"));
    }

    #[tokio::test]
    async fn nothing_is_claimed_while_the_sink_is_not_ready() {
        let (services, _owner, _id) = services_with_alert("acct_disp_idle").await;
        let s = sink(false, false);
        assert_eq!(
            dispatch_once(&services.outbox, s.as_ref(), &EventsConfig::for_tests()).await,
            0
        );
        assert_eq!(s.calls.load(Ordering::SeqCst), 0);
        s.ready.store(true, Ordering::SeqCst);
        assert_eq!(
            dispatch_once(&services.outbox, s.as_ref(), &EventsConfig::for_tests()).await,
            1
        );
    }

    #[tokio::test]
    async fn the_dispatcher_stops_on_cancel() {
        let (services, _owner, _id) = services_with_alert("acct_disp_stop").await;
        let shutdown = tokio_util::sync::CancellationToken::new();
        let task = tokio::spawn(run(
            services.outbox.clone(),
            sink(true, false),
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(EventsConfig::for_tests()),
            shutdown.clone(),
        ));
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("stops")
            .unwrap();
    }
}
