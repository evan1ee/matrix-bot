//! Reads and advances the outbox for the events runtime (`crate::events`).
//! Feature services write events themselves, through
//! `repositories::outbox::publish`, inside their own transactions.

use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::{next_delivery_step, DeliveryStep, EventId};
use crate::domain::webhooks::retry_delay;
use crate::infrastructure::postgres::repositories::outbox as repo;
pub use crate::infrastructure::postgres::repositories::outbox::{ClaimedDelivery, StoredEvent};

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

    pub async fn record_delivered(&self, delivery: &ClaimedDelivery) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        repo::mark_delivered(&mut *tx, delivery.id).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Schedules the next attempt, or fails the delivery after its last one.
    pub async fn record_failure(
        &self,
        delivery: &ClaimedDelivery,
        error: &str,
    ) -> Result<DeliveryStep, AppError> {
        let step = next_delivery_step(delivery.attempts, delivery.max_attempts);
        let mut tx = self.pool.begin().await?;
        match step {
            DeliveryStep::RetryAfterAttempts(n) => {
                repo::mark_retry(&mut *tx, delivery.id, Utc::now() + retry_delay(n), error).await?;
            }
            DeliveryStep::Fail => {
                repo::mark_failed(&mut *tx, delivery.id, error).await?;
            }
        }
        tx.commit().await?;
        Ok(step)
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
        assert_eq!(step, DeliveryStep::RetryAfterAttempts(0));
    }

    #[tokio::test]
    async fn a_failure_on_the_last_attempt_fails_the_delivery() {
        let (svc, _pool) = service_with_alert("acct_fail").await;
        let mut claimed = svc.claim_due(10, Duration::from_secs(60)).await.unwrap();
        claimed[0].attempts = claimed[0].max_attempts; // as if this was the 10th claim
        assert_eq!(
            svc.record_failure(&claimed[0], "down").await.unwrap(),
            DeliveryStep::Fail
        );
        assert!(svc
            .claim_due(10, Duration::from_secs(0))
            .await
            .unwrap()
            .is_empty());
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
