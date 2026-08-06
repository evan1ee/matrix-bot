use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::modules::reminder::models::{RecurrencePattern, Reminder};

/// Delivery attempts allowed before a reminder is marked `failed`.
///
/// Shared with `get_due_reminders` so the query and the scheduler cannot
/// disagree about when to stop retrying.
pub const MAX_DELIVERY_ATTEMPTS: i64 = 5;

pub struct ReminderRepository;

impl ReminderRepository {
    pub async fn create(
        pool: &SqlitePool,
        room_id: &str,
        sender_mxid: &str,
        sender_name: &str,
        message: &str,
        trigger_at: DateTime<Utc>,
        recurrence: RecurrencePattern,
    ) -> Result<Reminder> {
        let rec_str = recurrence.as_str();

        let res = sqlx::query(
            r#"
            INSERT INTO reminders (room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count)
            VALUES (?, ?, ?, ?, ?, ?, 'pending', 0)
            "#,
        )
        .bind(room_id)
        .bind(sender_mxid)
        .bind(sender_name)
        .bind(message)
        .bind(trigger_at)
        .bind(rec_str)
        .execute(pool)
        .await?;

        let id = res.last_insert_rowid();

        let reminder = sqlx::query_as::<_, Reminder>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count, created_at
            FROM reminders WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await?;

        Ok(reminder)
    }

    pub async fn count_user_pending(pool: &SqlitePool, sender_mxid: &str) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM reminders WHERE sender_mxid = ? AND status = 'pending'",
        )
        .bind(sender_mxid)
        .fetch_one(pool)
        .await?;

        Ok(count)
    }

    pub async fn get_due_reminders(pool: &SqlitePool) -> Result<Vec<Reminder>> {
        let now = Utc::now();
        let reminders = sqlx::query_as::<_, Reminder>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count, created_at
            FROM reminders WHERE status = 'pending' AND trigger_at <= ? AND retry_count < ?
            "#,
        )
        .bind(now)
        .bind(MAX_DELIVERY_ATTEMPTS)
        .fetch_all(pool)
        .await?;

        Ok(reminders)
    }

    pub async fn mark_triggered(pool: &SqlitePool, id: i64) -> Result<()> {
        sqlx::query("UPDATE reminders SET status = 'triggered' WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn mark_failed(pool: &SqlitePool, id: i64) -> Result<()> {
        sqlx::query("UPDATE reminders SET status = 'failed' WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn increment_retry(pool: &SqlitePool, id: i64, new_trigger_at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE reminders SET retry_count = retry_count + 1, trigger_at = ? WHERE id = ?")
            .bind(new_trigger_at)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn reschedule(pool: &SqlitePool, id: i64, new_trigger_at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE reminders SET trigger_at = ?, status = 'pending', retry_count = 0 WHERE id = ?")
            .bind(new_trigger_at)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn list_user_pending(
        pool: &SqlitePool,
        sender_mxid: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Reminder>> {
        let reminders = sqlx::query_as::<_, Reminder>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, retry_count, created_at
            FROM reminders WHERE sender_mxid = ? AND status = 'pending'
            ORDER BY trigger_at ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(sender_mxid)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(reminders)
    }

    pub async fn cancel(pool: &SqlitePool, sender_mxid: &str, id: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE reminders SET status = 'cancelled' WHERE id = ? AND sender_mxid = ? AND status = 'pending'",
        )
        .bind(id)
        .bind(sender_mxid)
        .execute(pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use chrono::Duration;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_reminder_crud_flow() {
        let pool = setup_test_db().await;
        let past_time = Utc::now() - Duration::seconds(10);

        let rem = ReminderRepository::create(
            &pool,
            "!room:a",
            "@bob:a",
            "bob",
            "Check oven",
            past_time,
            RecurrencePattern::None,
        )
        .await
        .unwrap();

        assert_eq!(rem.message, "Check oven");

        let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, rem.id);

        ReminderRepository::mark_triggered(&pool, rem.id).await.unwrap();

        let due_after = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert!(due_after.is_empty());
    }

    /// A failed delivery must neither be dropped nor retried forever.
    #[tokio::test]
    async fn test_retry_budget_is_bounded_and_never_silently_drops() {
        let pool = setup_test_db().await;
        let past = Utc::now() - Duration::seconds(10);

        let rem = ReminderRepository::create(
            &pool, "!room:a", "@bob:a", "bob", "Check oven", past, RecurrencePattern::None,
        )
        .await
        .unwrap();

        // Burn through the budget; each retry keeps it pending and due.
        for attempt in 0..MAX_DELIVERY_ATTEMPTS {
            let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
            assert_eq!(due.len(), 1, "still owed a delivery on attempt {}", attempt);
            assert_eq!(due[0].retry_count, attempt);

            ReminderRepository::increment_retry(&pool, rem.id, Utc::now() - Duration::seconds(1))
                .await
                .unwrap();
        }

        // Budget spent: the query stops offering it even though it is `pending`,
        // so it can never spin forever.
        let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert!(due.is_empty(), "exhausted reminder must stop being retried");

        // And the scheduler records the outcome rather than leaving it dangling.
        ReminderRepository::mark_failed(&pool, rem.id).await.unwrap();
        let status: (String,) = sqlx::query_as("SELECT status FROM reminders WHERE id = ?")
            .bind(rem.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status.0, "failed");
    }

    /// A successful delivery after retries clears the counter, so a later
    /// occurrence starts from a full budget.
    #[tokio::test]
    async fn test_reschedule_resets_the_retry_counter() {
        let pool = setup_test_db().await;
        let past = Utc::now() - Duration::seconds(10);

        let rem = ReminderRepository::create(
            &pool, "!room:a", "@bob:a", "bob", "Standup", past, RecurrencePattern::Daily,
        )
        .await
        .unwrap();

        ReminderRepository::increment_retry(&pool, rem.id, Utc::now() - Duration::seconds(1))
            .await
            .unwrap();
        let due = ReminderRepository::get_due_reminders(&pool).await.unwrap();
        assert_eq!(due[0].retry_count, 1);

        ReminderRepository::reschedule(&pool, rem.id, Utc::now() + Duration::days(1))
            .await
            .unwrap();

        let row: (i64, String) =
            sqlx::query_as("SELECT retry_count, status FROM reminders WHERE id = ?")
                .bind(rem.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, "pending");
    }
}
