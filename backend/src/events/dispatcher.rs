//! Delivers durable outbox copies (spec §5.4). One task per process;
//! `SKIP LOCKED` keeps several processes apart and the claim lease makes a
//! crashed claim reappear. Idles while the sink is not ready, so a Matrix
//! outage burns no attempts.

use std::sync::Arc;
use std::time::Instant;

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
            let handled = dispatch_once(&outbox, sink.as_ref(), &config, &shutdown).await;
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
///
/// The claim's lease is the whole batch's budget: each delivery's timeout is
/// capped by what is left of `lease - lease_margin`, and once the budget is
/// spent (or `shutdown` fires) the unsent rest is released, not failed.
pub(crate) async fn dispatch_once(
    outbox: &OutboxService,
    sink: &dyn DeliverySink,
    config: &EventsConfig,
    shutdown: &CancellationToken,
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
    let claimed_at = Instant::now();
    let budget = config.lease.saturating_sub(config.lease_margin);
    for (index, delivery) in claimed.iter().enumerate() {
        let remaining = budget.saturating_sub(claimed_at.elapsed());
        if shutdown.is_cancelled() || remaining.is_zero() {
            release_unsent(outbox, &claimed[index..]).await;
            break;
        }
        let timeout = config.deliver_timeout.min(remaining);
        let outcome = match tokio::time::timeout(timeout, sink.deliver(delivery)).await {
            Ok(result) => result,
            Err(_) => Err(format!("delivery timed out after {timeout:?}")),
        };
        record(outbox, delivery, outcome).await;
    }
    claimed.len()
}

async fn record(outbox: &OutboxService, delivery: &ClaimedDelivery, outcome: Result<(), String>) {
    let id = delivery.id.0;
    match outcome {
        Ok(()) => match outbox.record_delivered(delivery).await {
            Ok(true) => {
                info!(delivery_id = id, destination = %delivery.destination, "Delivered outbox event")
            }
            Ok(false) => {
                warn!(
                    delivery_id = id,
                    "Delivered but the claim had lost its lease; not recorded"
                )
            }
            Err(err) => {
                warn!(error = %err, delivery_id = id, "Delivered but could not record it; the lease will retry it")
            }
        },
        Err(msg) => match outbox.record_failure(delivery, &msg).await {
            Ok(Some(DeliveryStep::Fail)) => {
                warn!(delivery_id = id, attempts = delivery.attempts, error = %msg, "Outbox delivery failed permanently")
            }
            Ok(Some(DeliveryStep::RetryAfterAttempts(_))) => {
                warn!(delivery_id = id, attempts = delivery.attempts, error = %msg, "Outbox delivery failed; will retry")
            }
            Ok(None) => {
                warn!(delivery_id = id, error = %msg, "Outbox delivery failed but the claim had lost its lease; not recorded")
            }
            Err(err) => {
                warn!(error = %err, delivery_id = id, "Could not record a failed delivery; the lease will retry it")
            }
        },
    }
}

async fn release_unsent(outbox: &OutboxService, unsent: &[ClaimedDelivery]) {
    for delivery in unsent {
        match outbox.release(delivery).await {
            Ok(true) => {}
            Ok(false) => warn!(
                delivery_id = delivery.id.0,
                "Could not release an unsent claim: it had lost its lease"
            ),
            Err(err) => {
                warn!(error = %err, delivery_id = delivery.id.0, "Could not release an unsent claim; the lease will expire")
            }
        }
    }
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
    use crate::infrastructure::postgres::repositories::outbox as outbox_repo;

    async fn dispatch(services: &AppServices, s: &FakeSink) -> usize {
        dispatch_once(
            &services.outbox,
            s,
            &EventsConfig::for_tests(),
            &CancellationToken::new(),
        )
        .await
    }

    /// Two more alerts for the account's first webhook (three in total).
    async fn add_two_alerts(services: &AppServices, owner: &AccountId) {
        let hook = services.webhooks.list(owner).await.unwrap().remove(0);
        for _ in 0..2 {
            services
                .webhooks
                .enqueue_alert(
                    &hook,
                    "!room:example.org",
                    None,
                    "h",
                    "h",
                    crate::domain::webhooks::WebhookSeverity::Info,
                )
                .await
                .unwrap();
        }
    }

    struct FakeSink {
        ready: AtomicBool,
        fail: AtomicBool,
        calls: AtomicUsize,
        seen: Mutex<Vec<DeliveryId>>,
        /// Never returns (the send hangs until the dispatcher's timeout).
        hang: AtomicBool,
        /// Cancelled from inside the first delivery.
        cancel_during: Mutex<Option<CancellationToken>>,
    }

    #[async_trait::async_trait]
    impl DeliverySink for FakeSink {
        fn is_ready(&self) -> bool {
            self.ready.load(Ordering::SeqCst)
        }
        async fn deliver(&self, d: &ClaimedDelivery) -> Result<(), String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push(d.id);
            if self.hang.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if let Some(token) = self.cancel_during.lock().unwrap().take() {
                token.cancel();
            }
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
            hang: AtomicBool::new(false),
            cancel_during: Mutex::new(None),
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
        let n = dispatch_once(
            &services.outbox,
            s.as_ref(),
            &EventsConfig::for_tests(),
            &CancellationToken::new(),
        )
        .await;
        assert_eq!(n, 1);
        assert_eq!(*s.seen.lock().unwrap(), vec![id]);
        let hooks = services.webhooks.list(&owner).await.unwrap();
        assert_eq!(hooks[0].total_delivered_count, 1);
        assert_eq!(
            dispatch_once(
                &services.outbox,
                s.as_ref(),
                &EventsConfig::for_tests(),
                &CancellationToken::new()
            )
            .await,
            0
        );
    }

    #[tokio::test]
    async fn a_failed_delivery_is_retried_later_and_counted() {
        let (services, owner, _id) = services_with_alert("acct_disp_fail").await;
        let s = sink(true, true);
        assert_eq!(dispatch(&services, s.as_ref()).await, 1);
        // retry_delay(0) = 1 s on the database clock: scheduled, not due.
        assert_eq!(
            outbox_repo::delivery_probe_for(services.outbox.pool(), &owner)
                .await
                .unwrap(),
            vec![("pending".to_string(), 1, true)]
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
            dispatch_once(
                &services.outbox,
                s.as_ref(),
                &EventsConfig::for_tests(),
                &CancellationToken::new()
            )
            .await,
            0
        );
        assert_eq!(s.calls.load(Ordering::SeqCst), 0);
        s.ready.store(true, Ordering::SeqCst);
        assert_eq!(
            dispatch_once(
                &services.outbox,
                s.as_ref(),
                &EventsConfig::for_tests(),
                &CancellationToken::new()
            )
            .await,
            1
        );
    }

    /// Given a batch whose first send hangs past the lease budget, When the
    /// budget runs out, Then the send fails (attempt counted) and the unsent
    /// rest is released with its attempt restored, not failed or delayed.
    #[tokio::test]
    async fn a_spent_budget_releases_the_unsent_claims() {
        let (services, owner, _id) = services_with_alert("acct_disp_budget").await;
        add_two_alerts(&services, &owner).await;
        let s = sink(true, false);
        s.hang.store(true, Ordering::SeqCst);
        let config = EventsConfig {
            lease: Duration::from_millis(400),
            lease_margin: Duration::from_millis(100),
            deliver_timeout: Duration::from_millis(300),
            ..EventsConfig::for_tests()
        };
        let n = dispatch_once(
            &services.outbox,
            s.as_ref(),
            &config,
            &CancellationToken::new(),
        )
        .await;

        assert_eq!(n, 3);
        assert_eq!(s.calls.load(Ordering::SeqCst), 1, "only the first was sent");
        let probe = outbox_repo::delivery_probe_for(services.outbox.pool(), &owner)
            .await
            .unwrap();
        assert_eq!(
            probe[0],
            ("pending".to_string(), 1, true),
            "timed out: counted, backed off"
        );
        assert_eq!(
            probe[1],
            ("pending".to_string(), 0, false),
            "released: due, attempt restored"
        );
        assert_eq!(probe[2], ("pending".to_string(), 0, false));
    }

    /// Given shutdown during the first delivery, When the batch continues,
    /// Then the rest is released unsent and the first is recorded.
    #[tokio::test]
    async fn cancelling_mid_batch_releases_the_rest() {
        let (services, owner, _id) = services_with_alert("acct_disp_cancel_mid").await;
        add_two_alerts(&services, &owner).await;
        let shutdown = CancellationToken::new();
        let s = sink(true, false);
        *s.cancel_during.lock().unwrap() = Some(shutdown.clone());
        let n = dispatch_once(
            &services.outbox,
            s.as_ref(),
            &EventsConfig::for_tests(),
            &shutdown,
        )
        .await;

        assert_eq!(n, 3);
        assert_eq!(s.calls.load(Ordering::SeqCst), 1);
        let probe = outbox_repo::delivery_probe_for(services.outbox.pool(), &owner)
            .await
            .unwrap();
        assert_eq!(probe[0].0, "delivered");
        assert_eq!(probe[1], ("pending".to_string(), 0, false));
        assert_eq!(probe[2], ("pending".to_string(), 0, false));
    }

    /// Given a dispatcher idle in its wait, When shutdown fires, Then it stops.
    #[tokio::test]
    async fn the_dispatcher_stops_on_cancel_while_waiting() {
        let (services, _owner, _id) = services_with_alert("acct_disp_stop").await;
        let shutdown = CancellationToken::new();
        let s = sink(true, false);
        let task = tokio::spawn(run(
            services.outbox.clone(),
            s.clone(),
            Arc::new(tokio::sync::Notify::new()),
            Arc::new(EventsConfig {
                poll_interval: Duration::from_secs(30),
                ..EventsConfig::for_tests()
            }),
            shutdown.clone(),
        ));
        // Wait until the alert was delivered: the loop is now in its idle wait.
        tokio::time::timeout(Duration::from_secs(5), async {
            while s.calls.load(Ordering::SeqCst) == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("delivered");
        tokio::time::sleep(Duration::from_millis(100)).await;
        shutdown.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("stops while waiting")
            .unwrap();
    }
}
