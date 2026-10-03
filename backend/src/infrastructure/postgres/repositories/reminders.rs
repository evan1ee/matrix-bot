//! PostgreSQL repository for reminders.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::reminders::{
    NewReminder, RecurrencePattern, Reminder, ReminderSource, ReminderStatus, MAX_DELIVERY_ATTEMPTS,
};

#[derive(Debug, sqlx::FromRow)]
struct ReminderRow {
    id: i64,
    room_id: String,
    owner_id: uuid::Uuid,
    sender_name: String,
    message: String,
    trigger_at: DateTime<Utc>,
    recurrence: String,
    status: String,
    retry_count: i64,
    source_kind: Option<String>,
    source_id: Option<i64>,
    created_at: Option<DateTime<Utc>>,
}

impl From<ReminderRow> for Reminder {
    fn from(r: ReminderRow) -> Self {
        Self {
            id: r.id,
            room_id: r.room_id,
            owner_id: AccountId::from_uuid(r.owner_id),
            sender_name: r.sender_name,
            message: r.message,
            trigger_at: r.trigger_at,
            recurrence: RecurrencePattern::from_str_or_default(&r.recurrence),
            status: ReminderStatus::from_str_or_default(&r.status),
            retry_count: r.retry_count,
            source_kind: r
                .source_kind
                .as_deref()
                .and_then(ReminderSource::from_str_opt),
            source_id: r.source_id,
            created_at: r.created_at,
        }
    }
}

pub async fn create<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    input: &NewReminder,
) -> Result<Reminder, sqlx::Error> {
    let source_kind = input.source.map(|(k, _)| k.as_str());
    let source_id = input.source.map(|(_, id)| id);
    let recurrence_str = input.recurrence.as_str();

    let row = sqlx::query_as!(
        ReminderRow,
        r#"
        INSERT INTO reminders (
            room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id
        )
        VALUES ($1, $2, $3, $4, $5, $6, 'pending', 0, $7, $8)
        RETURNING
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        "#,
        input.room_id,
        owner_id.as_uuid(),
        input.owner_name,
        input.message,
        input.trigger_at,
        recurrence_str,
        source_kind,
        source_id
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find_owned<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<Reminder>, sqlx::Error> {
    let row = sqlx::query_as!(
        ReminderRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        FROM reminders
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_pending_for_source<'e>(
    conn: impl PgExecutor<'e>,
    source: ReminderSource,
    source_id: i64,
) -> Result<Option<Reminder>, sqlx::Error> {
    let row = sqlx::query_as!(
        ReminderRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        FROM reminders
        WHERE source_kind = $1 AND source_id = $2 AND status = 'pending'
        ORDER BY id DESC
        LIMIT 1
        "#,
        source.as_str(),
        source_id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn cancel_for_source<'e>(
    conn: impl PgExecutor<'e>,
    source: ReminderSource,
    source_id: i64,
) -> Result<Option<Reminder>, sqlx::Error> {
    let row = sqlx::query_as!(
        ReminderRow,
        r#"
        UPDATE reminders
        SET status = 'cancelled'
        WHERE id = (
            SELECT id FROM reminders
            WHERE source_kind = $1 AND source_id = $2 AND status = 'pending'
            ORDER BY id DESC
            LIMIT 1
        )
        RETURNING
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        "#,
        source.as_str(),
        source_id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn count_user_pending<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) FROM reminders WHERE owner_id = $1 AND status = 'pending'"#,
        owner_id.as_uuid()
    )
    .fetch_one(conn)
    .await?;

    Ok(count.unwrap_or(0))
}

pub async fn get_due_reminders<'e>(
    conn: impl PgExecutor<'e>,
    now: DateTime<Utc>,
) -> Result<Vec<Reminder>, sqlx::Error> {
    let rows = sqlx::query_as!(
        ReminderRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        FROM reminders
        WHERE status = 'pending' AND trigger_at <= $1 AND retry_count < $2
        ORDER BY trigger_at ASC
        "#,
        now,
        MAX_DELIVERY_ATTEMPTS
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn mark_triggered<'e>(conn: impl PgExecutor<'e>, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE reminders SET status = 'triggered' WHERE id = $1"#,
        id
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn mark_failed<'e>(conn: impl PgExecutor<'e>, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE reminders SET status = 'failed' WHERE id = $1"#,
        id
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Fails a reminder only while it is still pending at `trigger_at`, so a
/// snoozed or advanced reminder is never failed by an old job.
pub async fn mark_failed_at<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    trigger_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE reminders SET status = 'failed' \
         WHERE id = $1 AND trigger_at = $2 AND status = 'pending'",
        id,
        trigger_at
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Unscoped: only for the `reminder.fire` handler, which resolves a
/// system-generated row id. Never call with an id from a user.
pub async fn find_by_id<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
) -> Result<Option<Reminder>, sqlx::Error> {
    let row = sqlx::query_as!(
        ReminderRow,
        r#"
        SELECT
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        FROM reminders
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn record_failed_attempt<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    trigger_at: DateTime<Utc>,
    attempts: i64,
) -> Result<bool, sqlx::Error> {
    // retry_count mirrors the job's attempt number. trigger_at stays put: the
    // job row owns the backoff (R21), and the dedupe key <id>:<trigger_at>
    // must not change mid-retry.
    let result = sqlx::query!(
        "UPDATE reminders SET retry_count = $3 \
         WHERE id = $1 AND trigger_at = $2 AND status = 'pending'",
        id,
        trigger_at,
        attempts
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}
pub async fn reschedule<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    new_trigger_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE reminders SET trigger_at = $1, status = 'pending', retry_count = 0 WHERE id = $2"#,
        new_trigger_at,
        id
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn update_pending<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
    message: &str,
    trigger_at: DateTime<Utc>,
    recurrence: RecurrencePattern,
) -> Result<Option<Reminder>, sqlx::Error> {
    let recurrence_str = recurrence.as_str();
    let row = sqlx::query_as!(
        ReminderRow,
        r#"
        UPDATE reminders
        SET message = $1, trigger_at = $2, recurrence = $3, retry_count = 0
        WHERE id = $4 AND owner_id = $5 AND status = 'pending'
        RETURNING
            id, room_id, owner_id, sender_name, message, trigger_at,
            recurrence, status, retry_count, source_kind, source_id, created_at
        "#,
        message,
        trigger_at,
        recurrence_str,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn cancel<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE reminders SET status = 'cancelled' WHERE id = $1 AND owner_id = $2 AND status = 'pending'"#,
        id,
        owner_id.as_uuid()
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn force_cancel_owned<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"UPDATE reminders SET status = 'cancelled' WHERE id = $1 AND owner_id = $2"#,
        id,
        owner_id.as_uuid()
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn delete_history_item<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM reminders WHERE id = $1 AND owner_id = $2 AND status != 'pending'"#,
        id,
        owner_id.as_uuid()
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn clear_user_history<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM reminders WHERE owner_id = $1 AND status != 'pending'"#,
        owner_id.as_uuid()
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected())
}

pub async fn list_user_reminders<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    status_filter: Option<&str>,
    room_id: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Reminder>, sqlx::Error> {
    let is_pending = matches!(status_filter, Some("pending") | None);

    let rows = if is_pending {
        sqlx::query_as!(
            ReminderRow,
            r#"
            SELECT
                id, room_id, owner_id, sender_name, message, trigger_at,
                recurrence, status, retry_count, source_kind, source_id, created_at
            FROM reminders
            WHERE owner_id = $1
              AND ($2::text IS NULL OR room_id = $2)
              AND status = 'pending'
            ORDER BY trigger_at ASC
            LIMIT $3 OFFSET $4
            "#,
            owner_id.as_uuid(),
            room_id,
            limit,
            offset
        )
        .fetch_all(conn)
        .await?
    } else {
        let filter = status_filter.unwrap_or("pending");
        sqlx::query_as!(
            ReminderRow,
            r#"
            SELECT
                id, room_id, owner_id, sender_name, message, trigger_at,
                recurrence, status, retry_count, source_kind, source_id, created_at
            FROM reminders
            WHERE owner_id = $1
              AND ($2::text IS NULL OR room_id = $2)
              AND (
                ($3 = 'history' AND status != 'pending') OR
                ($3 = 'all') OR
                ($3 = 'triggered' AND status = 'triggered') OR
                ($3 = 'cancelled' AND status = 'cancelled') OR
                ($3 = 'failed' AND status = 'failed') OR
                (status = $3)
              )
            ORDER BY trigger_at DESC, id DESC
            LIMIT $4 OFFSET $5
            "#,
            owner_id.as_uuid(),
            room_id,
            filter,
            limit,
            offset
        )
        .fetch_all(conn)
        .await?
    };

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count_user_reminders<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    status_filter: Option<&str>,
    room_id: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let filter = status_filter.unwrap_or("pending");
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM reminders
        WHERE owner_id = $1
          AND ($2::text IS NULL OR room_id = $2)
          AND (
            ($3 = 'pending' AND status = 'pending') OR
            ($3 = 'history' AND status != 'pending') OR
            ($3 = 'all') OR
            ($3 = 'triggered' AND status = 'triggered') OR
            ($3 = 'cancelled' AND status = 'cancelled') OR
            ($3 = 'failed' AND status = 'failed') OR
            (status = $3)
          )
        "#,
        owner_id.as_uuid(),
        room_id,
        filter
    )
    .fetch_one(conn)
    .await?;

    Ok(count.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::time::now_at_storage_precision;
    use chrono::Duration;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_reminders_crud_flow() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_alice");

        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'alice', 'active')",
            owner.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();
        let now = now_at_storage_precision();

        let new = NewReminder {
            message: "Call mom".to_string(),
            trigger_at: now + Duration::hours(1),
            room_id: "!room:test".to_string(),
            owner_name: "alice".to_string(),
            recurrence: RecurrencePattern::None,
            source: None,
        };

        let created = create(&pool, &owner, &new).await.unwrap();
        assert_eq!(created.message, "Call mom");
        assert_eq!(created.status, ReminderStatus::Pending);

        let found = find_owned(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, created.id);

        let count = count_user_pending(&pool, &owner).await.unwrap();
        assert_eq!(count, 1);

        let updated = update_pending(
            &pool,
            &owner,
            created.id,
            "Call mom tonight",
            now + Duration::hours(2),
            RecurrencePattern::Daily,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.message, "Call mom tonight");
        assert_eq!(updated.recurrence, RecurrencePattern::Daily);

        let cancelled = cancel(&pool, &owner, created.id).await.unwrap();
        assert!(cancelled);

        let count_after = count_user_pending(&pool, &owner).await.unwrap();
        assert_eq!(count_after, 0);

        let deleted = delete_history_item(&pool, &owner, created.id)
            .await
            .unwrap();
        assert!(deleted);
    }

    fn new_reminder(now: DateTime<Utc>) -> NewReminder {
        NewReminder {
            message: "stretch".to_string(),
            trigger_at: now + Duration::hours(1),
            room_id: "!room:test".to_string(),
            owner_name: "alice".to_string(),
            recurrence: RecurrencePattern::None,
            source: None,
        }
    }

    /// Given a pending reminder, When a failed attempt is recorded for its
    /// trigger, Then retry_count = attempts and trigger_at is unchanged;
    /// for a different trigger_at nothing changes.
    #[tokio::test]
    async fn record_failed_attempt_keeps_the_trigger() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_retry_owner");
        crate::db::pool::seed_account(&pool, &owner).await;
        let created = create(&pool, &owner, &new_reminder(now_at_storage_precision()))
            .await
            .unwrap();

        assert!(
            record_failed_attempt(&pool, created.id, created.trigger_at, 2)
                .await
                .unwrap()
        );
        let after = find_owned(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.retry_count, 2);
        assert_eq!(after.trigger_at, created.trigger_at);
        assert_eq!(after.status, ReminderStatus::Pending);

        let stale = created.trigger_at + Duration::minutes(5);
        assert!(!record_failed_attempt(&pool, created.id, stale, 4)
            .await
            .unwrap());
        let unchanged = find_owned(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.retry_count, 2);
    }

    #[tokio::test]
    async fn find_by_id_ignores_ownership() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_find_owner");
        crate::db::pool::seed_account(&pool, &owner).await;
        let created = create(&pool, &owner, &new_reminder(now_at_storage_precision()))
            .await
            .unwrap();

        let found = find_by_id(&pool, created.id).await.unwrap().unwrap();
        assert_eq!(found.owner_id, owner);
        assert!(find_by_id(&pool, i64::MAX).await.unwrap().is_none());
    }
}
