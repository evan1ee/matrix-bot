//! Reads and advances the outbox for the events runtime (`crate::events`).
//! Feature services write events themselves, through
//! `repositories::outbox::publish`, inside their own transactions.

use std::time::Duration;

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::{next_delivery_step, DeliveryStep, EventId};
use crate::domain::webhooks::retry_delay;
use crate::infrastructure::postgres::repositories::outbox as repo;
pub use crate::infrastructure::postgres::repositories::outbox::{ClaimedDelivery, StoredEvent};
use crate::infrastructure::postgres::repositories::webhooks as webhooks_repo;

/// The webhook a `WebhookTriggered` delivery belongs to, whose counters
/// follow the delivery's outcome (C1).
fn webhook_id(delivery: &ClaimedDelivery) -> Option<String> {
    if delivery.event.event_type != "WebhookTriggered" {
        return None;
    }
    delivery.event.json["payload"]["id"]
        .as_str()
        .map(str::to_string)
}

#[derive(Clone)]
pub struct OutboxService {
    pool: PgPool,
}

impl OutboxService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Test-only: lets other modules' tests seed and inspect outbox rows.
    #[cfg(test)]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn load(&self, id: EventId) -> Result<Option<StoredEvent>, AppError> {
        Ok(repo::load(&self.pool, id).await?)
    }

    pub async fn load_since(
        &self,
        accounts: &[AccountId],
        after: EventId,
        limit: i64,
    ) -> Result<Vec<StoredEvent>, AppError> {
        Ok(repo::load_since(&self.pool, accounts, after, limit).await?)
    }

    pub async fn claim_due(
        &self,
        limit: i64,
        lease: Duration,
    ) -> Result<Vec<ClaimedDelivery>, AppError> {
        Ok(repo::claim_due(&self.pool, limit, lease).await?)
    }

    /// Returns whether this call made the transition; `false` means the lease
    /// was lost (the delivery was re-claimed or finished meanwhile) and
    /// nothing, counters included, changed.
    pub async fn record_delivered(&self, delivery: &ClaimedDelivery) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let recorded = repo::mark_delivered(&mut *tx, delivery.id, delivery.attempts).await?;
        if recorded {
            if let Some(webhook_id) = webhook_id(delivery) {
                webhooks_repo::increment_delivered(&mut *tx, &webhook_id).await?;
            }
        }
        tx.commit().await?;
        Ok(recorded)
    }

    /// Schedules the next attempt, or fails the delivery after its last one.
    /// `None` means the lease was lost and nothing changed.
    pub async fn record_failure(
        &self,
        delivery: &ClaimedDelivery,
        error: &str,
    ) -> Result<Option<DeliveryStep>, AppError> {
        let step = next_delivery_step(delivery.attempts, delivery.max_attempts);
        let mut tx = self.pool.begin().await?;
        let recorded = match step {
            DeliveryStep::RetryAfterAttempts(n) => {
                let delay =
                    Duration::from_secs(u64::try_from(retry_delay(n).num_seconds()).unwrap_or(0));
                repo::mark_retry(&mut *tx, delivery.id, delivery.attempts, delay, error).await?
            }
            DeliveryStep::Fail => {
                repo::mark_failed(&mut *tx, delivery.id, delivery.attempts, error).await?
            }
        };
        // C1: every failed attempt counts, retry or final.
        if recorded {
            if let Some(webhook_id) = webhook_id(delivery) {
                webhooks_repo::increment_failures(&mut *tx, &webhook_id).await?;
            }
        }
        tx.commit().await?;
        Ok(recorded.then_some(step))
    }

    /// Gives back a claim that was never sent, restoring its attempt.
    /// `false`: the lease was already lost.
    pub async fn release(&self, delivery: &ClaimedDelivery) -> Result<bool, AppError> {
        Ok(repo::release(&self.pool, delivery.id, delivery.attempts).await?)
    }

    pub async fn purge(&self, older_than: Duration) -> Result<u64, AppError> {
        Ok(repo::purge(&self.pool, older_than).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::events::{SystemEvent, WebhookAlert};
    use crate::infrastructure::postgres::repositories::outbox as repo;

    async fn service_with_alert(seed: &str) -> (OutboxService, sqlx::PgPool) {
        let pool = crate::db::pool::test_pool_sized(3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let mut tx = pool.begin().await.unwrap();
        repo::publish(
            &mut tx,
            &SystemEvent::WebhookTriggered(WebhookAlert {
                id: "whk_x".into(),
                owner_id: owner,
                room_id: "!r:1".into(),
                severity: "info".into(),
                title: None,
                html_body: "h".into(),
                plain_body: "h".into(),
            }),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        (OutboxService::new(pool.clone()), pool)
    }

    #[tokio::test]
    async fn a_failure_before_the_last_attempt_schedules_a_retry() {
        let (svc, _pool) = service_with_alert("acct_retry").await;
        let claimed = svc.claim_due(10, Duration::from_secs(60)).await.unwrap();
        let step = svc.record_failure(&claimed[0], "down").await.unwrap();
        assert_eq!(step, Some(DeliveryStep::RetryAfterAttempts(0)));
    }

    #[tokio::test]
    async fn a_failure_on_the_last_attempt_fails_the_delivery() {
        let (svc, _pool) = service_with_alert("acct_fail").await;
        let mut claimed = Vec::new();
        for _ in 0..10 {
            // A zero lease makes the row due again at once: the 10th claim.
            claimed = svc.claim_due(10, Duration::ZERO).await.unwrap();
        }
        assert_eq!(claimed[0].attempts, claimed[0].max_attempts);
        assert_eq!(
            svc.record_failure(&claimed[0], "down").await.unwrap(),
            Some(DeliveryStep::Fail)
        );
        assert!(svc
            .claim_due(10, Duration::from_secs(0))
            .await
            .unwrap()
            .is_empty());
    }

    /// Given a delivery whose lease expired and was re-claimed, When the first
    /// claimer records its outcome late, Then nothing changes: no state
    /// transition, no webhook counter.
    #[tokio::test]
    async fn a_stale_claim_cannot_overwrite_a_reclaim() {
        let pool = crate::db::pool::test_pool_sized(3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("acct_stale");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = crate::application::AppServices::new(&pool);
        let (hook, _) = services
            .webhooks
            .create(&owner, "ci", "!r:1")
            .await
            .unwrap();
        services
            .webhooks
            .enqueue_alert(
                &hook,
                "!r:1",
                None,
                "h",
                "h",
                crate::domain::webhooks::WebhookSeverity::Info,
            )
            .await
            .unwrap();
        let svc = &services.outbox;
        let first = svc.claim_due(10, Duration::from_millis(100)).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let second = svc.claim_due(10, Duration::from_secs(60)).await.unwrap();
        assert_eq!(second[0].attempts, 2);

        let _ = svc.record_failure(&first[0], "late").await;
        let _ = svc.record_delivered(&first[0]).await;

        assert_eq!(
            repo::delivery_probe_for(&pool, &owner).await.unwrap(),
            vec![("pending".to_string(), 2, true)],
            "still pending, still leased to the second claimer"
        );
        let hook = services.webhooks.get(&owner, &hook.id).await.unwrap();
        assert_eq!(hook.failure_count, 0);
        assert_eq!(hook.total_delivered_count, 0);
        // The live claimer can still finish it.
        svc.record_delivered(&second[0]).await.unwrap();
        let hook = services.webhooks.get(&owner, &hook.id).await.unwrap();
        assert_eq!(hook.total_delivered_count, 1);
    }

    #[tokio::test]
    async fn a_closed_pool_is_a_database_error_not_a_panic() {
        let (svc, pool) = service_with_alert("acct_closed").await;
        pool.close().await;
        assert!(matches!(
            svc.claim_due(10, Duration::from_secs(60)).await,
            Err(AppError::Database(_))
        ));
    }
}
