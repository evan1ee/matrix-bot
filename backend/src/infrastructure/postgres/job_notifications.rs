//! `LISTEN jobs`: wakes idle workers as soon as a job is inserted.
//!
//! One per process: `PgListener::connect_with` keeps one pool connection
//! for as long as it lives, and `recv` reconnects transparently after a
//! dropped socket.

use sqlx::postgres::PgListener;
use sqlx::PgPool;

use crate::infrastructure::postgres::repositories::jobs::JOBS_CHANNEL;

pub struct JobNotifications {
    listener: PgListener,
}

impl JobNotifications {
    pub async fn connect(pool: &PgPool) -> sqlx::Result<Self> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener.listen(JOBS_CHANNEL).await?;
        Ok(Self { listener })
    }

    /// Waits for the next notification (its payload is unused).
    pub async fn next(&mut self) -> sqlx::Result<()> {
        self.listener.recv().await.map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::jobs::{JobType, NewJob};
    use crate::infrastructure::postgres::repositories::jobs::enqueue;

    /// Given a listener on `jobs`, When a job is inserted, Then a notification
    /// arrives (the 0003 trigger). Only arrival is asserted: NOTIFY is
    /// database-wide and parallel tests add noise, never silence.
    #[tokio::test]
    async fn an_enqueue_notifies_listeners() {
        let pool = crate::db::pool::test_pool_sized(3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let mut listener = JobNotifications::connect(&pool).await.unwrap();
        enqueue(&pool, &NewJob::test_job(JobType::ReminderFire, None))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), listener.next())
            .await
            .expect("a notification within 5 s")
            .unwrap();
    }

    /// Given a listener on a private database, When a keyed job is inserted
    /// and then re-enqueued (ON CONFLICT DO NOTHING), Then exactly one
    /// notification arrives: the skipped insert sends none.
    #[tokio::test]
    async fn a_skipped_duplicate_sends_no_notification() {
        // NOTIFY is database-wide: a shared test database would let parallel
        // tests' inserts wake this listener, so "none" needs a private one.
        const DB: &str = "priv_skip_notify";
        let pool = crate::db::pool::test_pool_private_database(DB, 3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let mut listener = JobNotifications::connect(&pool).await.unwrap();
        let job = NewJob::test_job(JobType::PortfolioCheckAlerts, Some("k"));

        assert!(enqueue(&pool, &job).await.unwrap().is_some());
        tokio::time::timeout(Duration::from_secs(5), listener.next())
            .await
            .expect("the real insert notifies")
            .unwrap();

        assert!(enqueue(&pool, &job).await.unwrap().is_none());
        // A sentinel insert after the skip: if the skip had notified, its
        // notification is queued ahead of the sentinel's, so the next wake-up
        // is followed by a second one immediately.
        enqueue(
            &pool,
            &NewJob::test_job(JobType::PortfolioCheckAlerts, Some("sentinel")),
        )
        .await
        .unwrap()
        .unwrap();
        tokio::time::timeout(Duration::from_secs(5), listener.next())
            .await
            .expect("the sentinel insert notifies")
            .unwrap();
        let extra = tokio::time::timeout(Duration::from_millis(500), listener.next()).await;
        assert!(extra.is_err(), "the skipped duplicate must not notify");
        drop(listener);
        crate::db::pool::drop_private_database(pool, DB).await;
    }
}
