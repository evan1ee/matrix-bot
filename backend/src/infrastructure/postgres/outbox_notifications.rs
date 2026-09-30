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

    /// The next signal: a committed event, or the news that the connection
    /// was lost and re-established.
    pub async fn next(&mut self) -> sqlx::Result<OutboxSignal> {
        // `recv()` would swallow the `Ok(None)` of a silent reconnect (sqlx 0.9
        // reconnects eagerly on an IO-level loss), hiding the loss.
        let Some(n) = self.listener.try_recv().await? else {
            return Ok(OutboxSignal::Reconnected);
        };
        Ok(match parse_payload(n.payload()) {
            Some((id, account)) => OutboxSignal::Event(id, account),
            None => {
                warn!(
                    payload = n.payload(),
                    "Ignoring a malformed outbox notification"
                );
                OutboxSignal::Malformed
            }
        })
    }
}

/// What [`OutboxNotifications::next`] reports.
#[derive(Debug, PartialEq, Eq)]
pub enum OutboxSignal {
    Event(EventId, AccountId),
    /// The connection dropped and sqlx already re-LISTENed on a fresh one.
    /// NOTIFYs sent in between are gone, so the caller must replay.
    Reconnected,
    /// A payload that does not parse (logged, skipped).
    Malformed,
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
    use crate::infrastructure::postgres::advisory::terminate_other_backends;
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
                if let OutboxSignal::Event(id, account) = listener.next().await.unwrap() {
                    if id == published.event_id {
                        return (id, account);
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

    /// Given a listener whose connection is killed server-side, When it is
    /// polled, Then it reports `Reconnected` (sqlx reconnects silently inside
    /// `recv`, so this is the only place the loss is visible), and events
    /// published afterwards still arrive.
    #[tokio::test]
    async fn a_lost_connection_is_reported_and_the_listener_keeps_working() {
        let db = format!("outbox_notify_reconnect_{}", std::process::id());
        let db = db.as_str();
        let pool = crate::db::pool::test_pool_private_database(db, 4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("acct_reconnect");
        crate::db::pool::seed_account(&pool, &owner).await;
        let mut listener = OutboxNotifications::connect(&pool).await.unwrap();

        let killed = terminate_other_backends(&pool).await.unwrap();
        assert!(killed >= 1, "the listener's backend is among the others");
        let signal = tokio::time::timeout(Duration::from_secs(5), listener.next())
            .await
            .expect("the loss is reported within 5 s")
            .unwrap();
        assert_eq!(signal, OutboxSignal::Reconnected);

        let mut tx = pool.begin().await.unwrap();
        let published = publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: owner })
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let signal = tokio::time::timeout(Duration::from_secs(5), listener.next())
            .await
            .expect("the event arrives within 5 s")
            .unwrap();
        assert_eq!(signal, OutboxSignal::Event(published.event_id, owner));
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
