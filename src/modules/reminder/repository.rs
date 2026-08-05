use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::modules::reminder::models::{RecurrencePattern, Reminder};

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
            INSERT INTO reminders (room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status)
            VALUES (?, ?, ?, ?, ?, ?, 'pending')
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
            SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, created_at
            FROM reminders WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await?;

        Ok(reminder)
    }

    pub async fn get_due_reminders(pool: &SqlitePool) -> Result<Vec<Reminder>> {
        let now = Utc::now();
        let reminders = sqlx::query_as::<_, Reminder>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, created_at
            FROM reminders WHERE status = 'pending' AND trigger_at <= ?
            "#,
        )
        .bind(now)
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

    pub async fn reschedule(pool: &SqlitePool, id: i64, new_trigger_at: DateTime<Utc>) -> Result<()> {
        sqlx::query("UPDATE reminders SET trigger_at = ?, status = 'pending' WHERE id = ?")
            .bind(new_trigger_at)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn list_user_pending(pool: &SqlitePool, room_id: &str, sender_mxid: &str) -> Result<Vec<Reminder>> {
        let reminders = sqlx::query_as::<_, Reminder>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, message, trigger_at, recurrence, status, created_at
            FROM reminders WHERE room_id = ? AND sender_mxid = ? AND status = 'pending'
            ORDER BY trigger_at ASC
            "#,
        )
        .bind(room_id)
        .bind(sender_mxid)
        .fetch_all(pool)
        .await?;

        Ok(reminders)
    }

    pub async fn cancel(pool: &SqlitePool, room_id: &str, sender_mxid: &str, id: i64) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE reminders SET status = 'cancelled' WHERE id = ? AND room_id = ? AND sender_mxid = ? AND status = 'pending'",
        )
        .bind(id)
        .bind(room_id)
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
}
