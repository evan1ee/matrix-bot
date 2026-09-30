//! `LISTEN outbox`: one per process, shared by the SSE hub and the
//! delivery dispatcher (`events::listener`).

use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tracing::warn;

use crate::domain::accounts::AccountId;
use crate::domain::events::EventId;
use crate::infrastructure::postgres::repositories::outbox::OUTBOX_CHANNEL;

pub struct OutboxNotifications {
    listener: PgListener,
}

impl OutboxNotifications {
    pub async fn connect(pool: &PgPool) -> sqlx::Result<Self> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener.listen(OUTBOX_CHANNEL).await?;
        Ok(Self { listener })
    }

    /// The next committed event. `Ok(None)` is a malformed payload (logged).
    pub async fn next(&mut self) -> sqlx::Result<Option<(EventId, AccountId)>> {
        let n = self.listener.recv().await?;
        let parsed = parse_payload(n.payload());
        if parsed.is_none() {
            warn!(
                payload = n.payload(),
                "Ignoring a malformed outbox notification"
            );
        }
        Ok(parsed)
    }
}

fn parse_payload(payload: &str) -> Option<(EventId, AccountId)> {
    let (id, account) = payload.split_once(':')?;
    Some((
        EventId(id.parse().ok()?),
        AccountId::from_uuid(account.parse().ok()?),
    ))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::events::SystemEvent;
    use crate::infrastructure::postgres::repositories::outbox::publish;

    /// Given a listener, When an event commits, Then it receives
    /// `(event id, account)`.
    #[tokio::test]
    async fn a_committed_event_notifies_with_its_account() {
        let pool = crate::db::pool::test_pool_sized(3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("acct_notify");
        crate::db::pool::seed_account(&pool, &owner).await;
        let mut listener = OutboxNotifications::connect(&pool).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        let published = publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: owner })
            .await
            .unwrap();
        tx.commit().await.unwrap();

        // Other tests share the database-wide channel: skip foreign events.
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(n) = listener.next().await.unwrap() {
                    if n.0 == published.event_id {
                        return n;
                    }
                }
            }
        })
        .await
        .expect("notification within 5 s");
        assert_eq!(got.1, owner);
    }

    /// Given a listener on a private database, When a publish rolls back,
    /// Then no notification arrives (NOTIFY is delivered at commit only).
    #[tokio::test]
    async fn a_rolled_back_publish_sends_no_notification() {
        // NOTIFY is database-wide, so "none" needs a database of its own;
        // suffixed with the pid so concurrent runs cannot drop each other's.
        let db = format!("outbox_notify_rollback_{}", std::process::id());
        let db = db.as_str();
        let pool = crate::db::pool::test_pool_private_database(db, 3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("acct_rollback");
        crate::db::pool::seed_account(&pool, &owner).await;
        let mut listener = OutboxNotifications::connect(&pool).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: owner })
            .await
            .unwrap();
        tx.rollback().await.unwrap();

        let got = tokio::time::timeout(Duration::from_millis(500), listener.next()).await;
        assert!(got.is_err(), "a rollback must not notify");
        drop(listener);
        crate::db::pool::drop_private_database(pool, db).await;
    }

    #[test]
    fn payloads_parse_or_are_rejected() {
        let uuid = uuid::Uuid::new_v4();
        assert_eq!(
            parse_payload(&format!("42:{uuid}")),
            Some((EventId(42), AccountId::from_uuid(uuid)))
        );
        assert_eq!(parse_payload("42"), None);
        assert_eq!(parse_payload("x:y"), None);
    }
}
