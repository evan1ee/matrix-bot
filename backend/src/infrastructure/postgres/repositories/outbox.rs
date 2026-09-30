//! The transactional outbox (`outbox_events`, `outbox_deliveries`, 0004).
//!
//! `publish` is the only writer of events and takes a `Transaction`, so an
//! event can only ever be written together with the state change it
//! describes. Deliveries are claimed with a lease (`available_at` pushed
//! forward), so a crashed dispatcher's claim simply expires.

use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, Postgres, Transaction};

use crate::domain::accounts::AccountId;
use crate::domain::events::{DeliveryId, EventId, SystemEvent};
use crate::domain::jobs::truncate_error;

/// The channel the 0004 trigger NOTIFYs with `<event id>:<account uuid>`.
pub const OUTBOX_CHANNEL: &str = "outbox";

#[derive(Debug, Clone)]
pub struct StoredEvent {
    pub id: EventId,
    pub account_id: AccountId,
    pub event_type: String,
    /// The serialised `SystemEvent` (`{type, payload}`), exactly as SSE sends it.
    pub json: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ClaimedDelivery {
    pub id: DeliveryId,
    pub destination: String,
    /// Includes the claim that returned this row.
    pub attempts: i32,
    pub max_attempts: i32,
    pub event: StoredEvent,
}

#[derive(Debug, Clone)]
pub struct PublishedEvent {
    pub event_id: EventId,
    pub deliveries: Vec<DeliveryId>,
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

pub async fn publish(
    tx: &mut Transaction<'_, Postgres>,
    event: &SystemEvent,
) -> sqlx::Result<PublishedEvent> {
    let json = serde_json::to_value(event).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
    let event_id = sqlx::query_scalar!(
        "INSERT INTO outbox_events (account_id, event_type, payload) VALUES ($1, $2, $3) RETURNING id",
        event.owner_id().as_uuid(),
        event.event_type(),
        json,
    )
    .fetch_one(&mut **tx)
    .await?;

    let mut deliveries = Vec::new();
    for destination in event.destinations() {
        let id = sqlx::query_scalar!(
            "INSERT INTO outbox_deliveries (event_id, destination) VALUES ($1, $2) RETURNING id",
            event_id,
            destination.as_key(),
        )
        .fetch_one(&mut **tx)
        .await?;
        deliveries.push(DeliveryId(id));
    }
    Ok(PublishedEvent {
        event_id: EventId(event_id),
        deliveries,
    })
}

struct EventRow {
    id: i64,
    account_id: uuid::Uuid,
    event_type: String,
    payload: serde_json::Value,
}

impl From<EventRow> for StoredEvent {
    fn from(r: EventRow) -> Self {
        Self {
            id: EventId(r.id),
            account_id: AccountId::from_uuid(r.account_id),
            event_type: r.event_type,
            json: r.payload,
        }
    }
}

pub async fn load(ex: impl PgExecutor<'_>, id: EventId) -> sqlx::Result<Option<StoredEvent>> {
    let row = sqlx::query_as!(
        EventRow,
        "SELECT id, account_id, event_type, payload FROM outbox_events WHERE id = $1",
        id.0
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn load_since(
    ex: impl PgExecutor<'_>,
    accounts: &[AccountId],
    after: EventId,
    limit: i64,
) -> sqlx::Result<Vec<StoredEvent>> {
    let uuids: Vec<uuid::Uuid> = accounts.iter().map(|a| *a.as_uuid()).collect();
    let rows = sqlx::query_as!(
        EventRow,
        r#"
        SELECT id, account_id, event_type, payload FROM outbox_events
        WHERE id > $1 AND account_id = ANY($2)
        ORDER BY id
        LIMIT $3
        "#,
        after.0,
        &uuids,
        limit
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

struct ClaimedRow {
    id: i64,
    destination: String,
    attempts: i32,
    max_attempts: i32,
    event_id: i64,
    account_id: uuid::Uuid,
    event_type: String,
    payload: serde_json::Value,
}

impl From<ClaimedRow> for ClaimedDelivery {
    fn from(r: ClaimedRow) -> Self {
        Self {
            id: DeliveryId(r.id),
            destination: r.destination,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            event: StoredEvent {
                id: EventId(r.event_id),
                account_id: AccountId::from_uuid(r.account_id),
                event_type: r.event_type,
                json: r.payload,
            },
        }
    }
}

pub async fn claim_due(
    ex: impl PgExecutor<'_>,
    limit: i64,
    lease: Duration,
) -> sqlx::Result<Vec<ClaimedDelivery>> {
    let rows = sqlx::query_as!(
        ClaimedRow,
        r#"
        WITH due AS (
            SELECT id FROM outbox_deliveries
            WHERE status = 'pending' AND available_at <= NOW()
            ORDER BY available_at, id
            FOR UPDATE SKIP LOCKED
            LIMIT $1
        )
        UPDATE outbox_deliveries d
        SET attempts = d.attempts + 1,
            available_at = NOW() + make_interval(secs => $2)
        FROM due, outbox_events e
        WHERE d.id = due.id AND e.id = d.event_id
        RETURNING d.id, d.destination, d.attempts, d.max_attempts,
                  e.id AS event_id, e.account_id, e.event_type, e.payload
        "#,
        limit,
        secs(lease)
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn mark_delivered(ex: impl PgExecutor<'_>, id: DeliveryId) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET status = 'delivered', delivered_at = NOW() WHERE id = $1 AND status = 'pending'",
        id.0
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn mark_retry(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    available_at: DateTime<Utc>,
    error: &str,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET available_at = $2, last_error = $3 WHERE id = $1 AND status = 'pending'",
        id.0,
        available_at,
        truncate_error(error)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn mark_failed(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    error: &str,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET status = 'failed', last_error = $2 WHERE id = $1 AND status = 'pending'",
        id.0,
        truncate_error(error)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn purge(ex: impl PgExecutor<'_>, older_than: Duration) -> sqlx::Result<u64> {
    let r = sqlx::query!(
        r#"
        DELETE FROM outbox_events e
        WHERE e.created_at < NOW() - make_interval(secs => $1)
          AND NOT EXISTS (
              SELECT 1 FROM outbox_deliveries d
              WHERE d.event_id = e.id AND d.status = 'pending')
        "#,
        secs(older_than)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected())
}

#[cfg(test)]
pub async fn event_types_for(
    ex: impl PgExecutor<'_>,
    account: &AccountId,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        "SELECT event_type FROM outbox_events WHERE account_id = $1 ORDER BY id",
        account.as_uuid()
    )
    .fetch_all(ex)
    .await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool_sized};
    use crate::domain::accounts::AccountId;
    use crate::domain::events::{EventId, SystemEvent, WebhookAlert};

    async fn pool_with(accounts: &[&str]) -> (sqlx::PgPool, Vec<AccountId>) {
        let pool = test_pool_sized(4).await;
        run_migrations_unlocked(&pool).await;
        let ids: Vec<AccountId> = accounts.iter().map(|s| AccountId::from_seed(s)).collect();
        for id in &ids {
            seed_account(&pool, id).await;
        }
        (pool, ids)
    }

    fn alert(owner: AccountId, room: &str) -> SystemEvent {
        SystemEvent::WebhookTriggered(WebhookAlert {
            id: "whk_1".into(),
            owner_id: owner,
            room_id: room.into(),
            severity: "info".into(),
            title: None,
            html_body: "<b>h</b>".into(),
            plain_body: "h".into(),
        })
    }

    /// Given an event with a durable destination, When it is published and
    /// committed, Then the event and one pending delivery exist.
    #[tokio::test]
    async fn publish_writes_the_event_and_its_deliveries() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut tx = pool.begin().await.unwrap();
        let published = publish(&mut tx, &alert(ids[0], "!r:1")).await.unwrap();
        tx.commit().await.unwrap();

        assert_eq!(published.deliveries.len(), 1);
        let stored = load(&pool, published.event_id).await.unwrap().unwrap();
        assert_eq!(stored.account_id, ids[0]);
        assert_eq!(stored.event_type, "WebhookTriggered");
        assert_eq!(stored.json["payload"]["room_id"], "!r:1");
        assert_eq!(
            event_types_for(&pool, &ids[0]).await.unwrap(),
            vec!["WebhookTriggered"]
        );
    }

    /// Given a publish inside a transaction, When it rolls back, Then
    /// neither the event nor its delivery survives.
    #[tokio::test]
    async fn a_rolled_back_publish_leaves_nothing() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &alert(ids[0], "!r:1")).await.unwrap();
        tx.rollback().await.unwrap();

        assert!(event_types_for(&pool, &ids[0]).await.unwrap().is_empty());
        assert!(claim_due(&pool, 10, Duration::from_secs(60))
            .await
            .unwrap()
            .is_empty());
    }

    /// Two concurrent claimers never receive the same delivery.
    #[tokio::test]
    async fn concurrent_claims_are_disjoint() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        for n in 0..6 {
            let mut tx = pool.begin().await.unwrap();
            publish(&mut tx, &alert(ids[0], &format!("!r:{n}")))
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }
        let (a, b) = tokio::join!(
            claim_due(&pool, 4, Duration::from_secs(60)),
            claim_due(&pool, 4, Duration::from_secs(60)),
        );
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.len() + b.len(), 6);
        for d in &a {
            assert!(
                b.iter().all(|o| o.id != d.id),
                "delivery {:?} claimed twice",
                d.id
            );
        }
        assert!(a.iter().chain(&b).all(|d| d.attempts == 1));
    }

    /// A claimed delivery is invisible until its lease expires, then it is
    /// claimable again with its attempt counted.
    #[tokio::test]
    async fn an_expired_lease_is_reclaimed() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &alert(ids[0], "!r:1")).await.unwrap();
        tx.commit().await.unwrap();

        let first = claim_due(&pool, 10, Duration::from_millis(200))
            .await
            .unwrap();
        assert_eq!(first.len(), 1);
        assert!(claim_due(&pool, 10, Duration::from_secs(60))
            .await
            .unwrap()
            .is_empty());
        tokio::time::sleep(Duration::from_millis(400)).await;
        let again = claim_due(&pool, 10, Duration::from_secs(60)).await.unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].attempts, 2);
    }

    #[tokio::test]
    async fn marks_move_a_delivery_out_of_the_queue() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &alert(ids[0], "!r:1")).await.unwrap();
        publish(&mut tx, &alert(ids[0], "!r:2")).await.unwrap();
        publish(&mut tx, &alert(ids[0], "!r:3")).await.unwrap();
        tx.commit().await.unwrap();
        let claimed = claim_due(&pool, 10, Duration::from_millis(1))
            .await
            .unwrap();

        assert!(mark_delivered(&pool, claimed[0].id).await.unwrap());
        assert!(mark_failed(&pool, claimed[1].id, "boom").await.unwrap());
        assert!(mark_retry(
            &pool,
            claimed[2].id,
            Utc::now() - chrono::Duration::seconds(1),
            "later"
        )
        .await
        .unwrap());

        tokio::time::sleep(Duration::from_millis(20)).await;
        let next = claim_due(&pool, 10, Duration::from_secs(60)).await.unwrap();
        assert_eq!(next.len(), 1, "only the retried delivery is due");
        assert_eq!(next[0].id, claimed[2].id);
    }

    /// Retention deletes old finished events but never one still pending.
    #[tokio::test]
    async fn purge_keeps_events_with_pending_deliveries() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: ids[0] })
            .await
            .unwrap();
        publish(&mut tx, &alert(ids[0], "!r:1")).await.unwrap();
        tx.commit().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert_eq!(purge(&pool, Duration::from_millis(10)).await.unwrap(), 1);
        assert_eq!(
            event_types_for(&pool, &ids[0]).await.unwrap(),
            vec!["WebhookTriggered"]
        );
    }

    #[tokio::test]
    async fn load_since_is_scoped_to_the_given_accounts() {
        let (pool, ids) = pool_with(&["acct_a", "acct_b"]).await;
        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: ids[0] })
            .await
            .unwrap();
        publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: ids[1] })
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let mine = load_since(&pool, &ids[..1], EventId(0), 100).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].account_id, ids[0]);
    }
}
