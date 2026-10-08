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
use crate::domain::events::{DeliveryId, Destination, EventId, SystemEvent, WebhookAlert};
use crate::domain::jobs::truncate_error;
use crate::domain::webhooks::WebhookOutboxItem;

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
    // The NEWEST `limit` rows, oldest first: a replay bounded by `limit`
    // must drop the stale end of the gap, not the fresh one.
    let rows = sqlx::query_as!(
        EventRow,
        r#"
        SELECT id AS "id!", account_id AS "account_id!", event_type AS "event_type!", payload AS "payload!"
        FROM (
            SELECT id, account_id, event_type, payload FROM outbox_events
            WHERE id > $1 AND account_id = ANY($2)
            ORDER BY id DESC
            LIMIT $3
        ) newest
        ORDER BY id
        "#,
        after.0,
        &uuids,
        limit
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// The highest event id, 0 when the outbox is empty.
pub async fn max_event_id(ex: impl PgExecutor<'_>) -> sqlx::Result<EventId> {
    let max = sqlx::query_scalar!(r#"SELECT COALESCE(MAX(id), 0) AS "max!" FROM outbox_events"#)
        .fetch_one(ex)
        .await?;
    Ok(EventId(max))
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

// Every mark below is fenced by the `attempts` the claim returned: a claim
// that outlived its lease (and was re-claimed, which bumps `attempts`)
// changes nothing. `false` means the lease was lost.

pub async fn mark_delivered(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    claimed_attempts: i32,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET status = 'delivered', delivered_at = NOW() WHERE id = $1 AND status = 'pending' AND attempts = $2",
        id.0,
        claimed_attempts
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Schedules the next attempt `delay` from now on the database clock, the
/// same one `claim_due` compares against.
pub async fn mark_retry(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    claimed_attempts: i32,
    delay: Duration,
    error: &str,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET available_at = NOW() + make_interval(secs => $3), last_error = $4 WHERE id = $1 AND status = 'pending' AND attempts = $2",
        id.0,
        claimed_attempts,
        secs(delay),
        truncate_error(error)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

pub async fn mark_failed(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    claimed_attempts: i32,
    error: &str,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET status = 'failed', last_error = $3 WHERE id = $1 AND status = 'pending' AND attempts = $2",
        id.0,
        claimed_attempts,
        truncate_error(error)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Defers a delivery whose channel is offline or unavailable:
/// restores its attempt counter (`attempts = attempts - 1`) and schedules
/// it `delay` seconds into the future so it does not busy-loop.
pub async fn defer(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    claimed_attempts: i32,
    delay: Duration,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET attempts = attempts - 1, available_at = NOW() + make_interval(secs => $3) WHERE id = $1 AND status = 'pending' AND attempts = $2",
        id.0,
        claimed_attempts,
        secs(delay)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Gives back a claim that was never sent (batch budget spent, shutdown):
/// the attempt is not burned and the row is due at once.
pub async fn release(
    ex: impl PgExecutor<'_>,
    id: DeliveryId,
    claimed_attempts: i32,
) -> sqlx::Result<bool> {
    let r = sqlx::query!(
        "UPDATE outbox_deliveries SET attempts = attempts - 1, available_at = NOW() WHERE id = $1 AND status = 'pending' AND attempts = $2",
        id.0,
        claimed_attempts
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected() == 1)
}

/// Stops the pending alerts of a deleted webhook (the pre-outbox `ON DELETE
/// CASCADE` semantics). Scoped to the owner: the webhook id is only a JSON
/// field of the event, so ownership must come from `account_id`.
pub async fn fail_pending_webhook_deliveries(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    webhook_id: &str,
    reason: &str,
) -> sqlx::Result<u64> {
    let r = sqlx::query!(
        r#"
        UPDATE outbox_deliveries d SET status = 'failed', last_error = $3
        FROM outbox_events e
        WHERE d.event_id = e.id
          AND e.account_id = $1
          AND e.event_type = 'WebhookTriggered'
          AND e.payload -> 'payload' ->> 'id' = $2
          AND d.status = 'pending'
        "#,
        owner.as_uuid(),
        webhook_id,
        truncate_error(reason)
    )
    .execute(ex)
    .await?;
    Ok(r.rows_affected())
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

struct WebhookLogRow {
    id: i64,
    account_id: uuid::Uuid,
    payload: serde_json::Value,
    destination: String,
    status: String,
    attempts: i32,
    max_attempts: i32,
    available_at: DateTime<Utc>,
    last_error: Option<String>,
    created_at: DateTime<Utc>,
    delivered_at: Option<DateTime<Utc>>,
}

impl TryFrom<WebhookLogRow> for WebhookOutboxItem {
    type Error = sqlx::Error;

    /// A payload that is not a `WebhookAlert` is a decode error, not a panic.
    fn try_from(r: WebhookLogRow) -> sqlx::Result<Self> {
        let alert: WebhookAlert = serde_json::from_value(r.payload["payload"].clone())
            .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let room_id = match Destination::parse(&r.destination) {
            Some(Destination::Conversation(address)) => address.to_storage(),
            None => alert.room_id.clone(),
        };
        Ok(Self {
            id: r.id.to_string(),
            webhook_id: alert.id,
            owner_id: AccountId::from_uuid(r.account_id),
            room_id,
            severity: alert.severity,
            title: alert.title,
            html_body: alert.html_body,
            plain_body: alert.plain_body,
            status: r.status,
            attempts: i64::from(r.attempts),
            max_attempts: i64::from(r.max_attempts),
            next_attempt_at: r.available_at,
            last_error: r.last_error,
            created_at: r.created_at,
            delivered_at: r.delivered_at,
        })
    }
}

/// The delivery log of one webhook, newest first: the alert (from its event)
/// joined with its delivery state. Bounds match the log reads this replaced.
pub async fn list_webhook_logs(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    webhook_id: &str,
    limit: i64,
    offset: i64,
) -> sqlx::Result<Vec<WebhookOutboxItem>> {
    let rows = sqlx::query_as!(
        WebhookLogRow,
        r#"
        SELECT d.id, e.account_id, e.payload, d.destination, d.status, d.attempts,
               d.max_attempts, d.available_at, d.last_error, e.created_at, d.delivered_at
        FROM outbox_events e
        JOIN outbox_deliveries d ON d.event_id = e.id
        WHERE e.account_id = $1
          AND e.event_type = 'WebhookTriggered'
          AND e.payload -> 'payload' ->> 'id' = $2
        ORDER BY e.created_at DESC, d.id DESC
        LIMIT $3 OFFSET $4
        "#,
        owner.as_uuid(),
        webhook_id,
        limit.clamp(1, 100),
        offset.max(0)
    )
    .fetch_all(ex)
    .await?;
    rows.into_iter().map(WebhookOutboxItem::try_from).collect()
}

pub async fn count_webhook_logs(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    webhook_id: &str,
) -> sqlx::Result<i64> {
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) FROM outbox_events e
        JOIN outbox_deliveries d ON d.event_id = e.id
        WHERE e.account_id = $1
          AND e.event_type = 'WebhookTriggered'
          AND e.payload -> 'payload' ->> 'id' = $2
        "#,
        owner.as_uuid(),
        webhook_id
    )
    .fetch_one(ex)
    .await?;
    Ok(count.unwrap_or(0))
}

/// Test-only: makes every later `outbox_events` insert in this test's schema
/// raise, so a service's real `publish` fails and the atomicity of "state
/// change + event" is observable. The trigger lives in the schema on the
/// pool's `search_path`, so it cannot affect another test.
#[cfg(test)]
pub async fn fail_every_publish(pool: &sqlx::PgPool) {
    sqlx::raw_sql(
        "CREATE FUNCTION outbox_test_fail() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'outbox publish forced to fail'; END $$; \
         CREATE TRIGGER outbox_test_fail BEFORE INSERT ON outbox_events \
         FOR EACH ROW EXECUTE FUNCTION outbox_test_fail()",
    )
    .execute(pool)
    .await
    .expect("install the failing-publish trigger");
}

/// Test-only: the delivery states of one account's events, oldest first.
#[cfg(test)]
pub async fn delivery_statuses_for(
    ex: impl PgExecutor<'_>,
    account: &AccountId,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        r#"SELECT d.status FROM outbox_deliveries d JOIN outbox_events e ON e.id = d.event_id
           WHERE e.account_id = $1 ORDER BY d.id"#,
        account.as_uuid()
    )
    .fetch_all(ex)
    .await
}

/// Test-only: `(status, attempts, available_at > now)` of each of one
/// account's deliveries, oldest first.
#[cfg(test)]
pub async fn delivery_probe_for(
    ex: impl PgExecutor<'_>,
    account: &AccountId,
) -> sqlx::Result<Vec<(String, i32, bool)>> {
    let rows = sqlx::query!(
        r#"SELECT d.status, d.attempts, (d.available_at > NOW()) AS "later!"
           FROM outbox_deliveries d JOIN outbox_events e ON e.id = d.event_id
           WHERE e.account_id = $1 ORDER BY d.id"#,
        account.as_uuid()
    )
    .fetch_all(ex)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.status, r.attempts, r.later))
        .collect())
}

/// Test-only: seconds between each of one account's events being created and
/// its delivery next becoming due, oldest first.
///
/// Unlike [`delivery_probe_for`]'s `available_at > NOW()`, this does not shrink
/// as the test runs: a backoff of `d` seconds always reads `>= d`, however
/// slowly the machine reaches the assertion. Use it to prove a delivery was
/// pushed back by at least a given delay; use the probe to prove it is due.
#[cfg(test)]
pub async fn delivery_backoff_for(
    ex: impl PgExecutor<'_>,
    account: &AccountId,
) -> sqlx::Result<Vec<f64>> {
    let rows = sqlx::query!(
        r#"SELECT EXTRACT(EPOCH FROM (d.available_at - e.created_at))::float8 AS "secs!"
           FROM outbox_deliveries d JOIN outbox_events e ON e.id = d.event_id
           WHERE e.account_id = $1 ORDER BY d.id"#,
        account.as_uuid()
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(|r| r.secs).collect())
}

/// Test-only: the destination keys of one account's deliveries, oldest first.
#[cfg(test)]
pub async fn delivery_destinations_for(
    ex: impl PgExecutor<'_>,
    account: &AccountId,
) -> sqlx::Result<Vec<String>> {
    sqlx::query_scalar!(
        "SELECT d.destination FROM outbox_deliveries d JOIN outbox_events e ON e.id = d.event_id
         WHERE e.account_id = $1 ORDER BY d.id",
        account.as_uuid()
    )
    .fetch_all(ex)
    .await
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

        assert!(mark_delivered(&pool, claimed[0].id, 1).await.unwrap());
        assert!(mark_failed(&pool, claimed[1].id, 1, "boom").await.unwrap());
        assert!(mark_retry(&pool, claimed[2].id, 1, Duration::ZERO, "later")
            .await
            .unwrap());

        tokio::time::sleep(Duration::from_millis(20)).await;
        let next = claim_due(&pool, 10, Duration::from_secs(60)).await.unwrap();
        assert_eq!(next.len(), 1, "only the retried delivery is due");
        assert_eq!(next[0].id, claimed[2].id);
    }

    /// A mark carrying an older claim's attempt count is refused; a release
    /// restores the attempt and makes the row due again.
    #[tokio::test]
    async fn marks_are_fenced_by_the_claimed_attempts() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut tx = pool.begin().await.unwrap();
        publish(&mut tx, &alert(ids[0], "!r:1")).await.unwrap();
        tx.commit().await.unwrap();
        let first = claim_due(&pool, 10, Duration::ZERO).await.unwrap();
        let second = claim_due(&pool, 10, Duration::from_secs(60)).await.unwrap();
        assert_eq!((first[0].attempts, second[0].attempts), (1, 2));
        let id = first[0].id;

        assert!(!mark_delivered(&pool, id, 1).await.unwrap());
        assert!(!mark_failed(&pool, id, 1, "x").await.unwrap());
        assert!(!mark_retry(&pool, id, 1, Duration::ZERO, "x").await.unwrap());
        assert!(!release(&pool, id, 1).await.unwrap());
        assert_eq!(
            delivery_probe_for(&pool, &ids[0]).await.unwrap(),
            vec![("pending".to_string(), 2, true)]
        );

        assert!(release(&pool, id, 2).await.unwrap());
        assert_eq!(
            delivery_probe_for(&pool, &ids[0]).await.unwrap(),
            vec![("pending".to_string(), 1, false)],
            "the released attempt was restored and the row is due"
        );
        let again = claim_due(&pool, 10, Duration::from_secs(60)).await.unwrap();
        assert_eq!(again[0].attempts, 2);
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

    /// With more rows than `limit`, the newest ones come back, ascending.
    #[tokio::test]
    async fn load_since_returns_the_newest_rows_when_over_the_limit() {
        let (pool, ids) = pool_with(&["acct_a"]).await;
        let mut published = Vec::new();
        for _ in 0..5 {
            let mut tx = pool.begin().await.unwrap();
            let p = publish(&mut tx, &SystemEvent::AiConfigUpdated { owner_id: ids[0] })
                .await
                .unwrap();
            tx.commit().await.unwrap();
            published.push(p.event_id);
        }

        let got = load_since(&pool, &ids, EventId(0), 2).await.unwrap();
        let got: Vec<EventId> = got.into_iter().map(|e| e.id).collect();
        assert_eq!(got, published[3..].to_vec());
        assert_eq!(max_event_id(&pool).await.unwrap(), published[4]);
    }
}
