//! Reminder lookups keyed by the entity a reminder was derived from, kept
//! apart from `reminders.rs` to hold that file under the line cap.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::reminders::ReminderSource;

/// Room of the most recent reminder of any status for `source`/`source_id`
/// owned by `owner_id`.
pub async fn last_room_for_source<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    source: ReminderSource,
    source_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT room_id FROM reminders
        WHERE owner_id = $1 AND source_kind = $2 AND source_id = $3
        ORDER BY id DESC
        LIMIT 1
        "#,
        owner_id.as_uuid(),
        source.as_str(),
        source_id
    )
    .fetch_optional(conn)
    .await
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::domain::reminders::{NewReminder, RecurrencePattern};
    use crate::infrastructure::postgres::repositories::reminders;

    /// Given owner A's reminder for a todo, When owner B looks up that todo
    /// id, Then B gets nothing while A gets the room.
    #[tokio::test]
    async fn last_room_for_source_is_owner_isolated() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let alice = AccountId::from_seed("rs_alice");
        let bob = AccountId::from_seed("rs_bob");
        crate::db::pool::seed_account(&pool, &alice).await;
        crate::db::pool::seed_account(&pool, &bob).await;
        let new = NewReminder {
            message: "Task #7 is due".to_string(),
            trigger_at: crate::utils::time::now_at_storage_precision() + Duration::hours(1),
            room_id: "!alice:room".to_string(),
            owner_name: "alice".to_string(),
            recurrence: RecurrencePattern::None,
            source: Some((ReminderSource::Todo, 7)),
        };
        reminders::create(&pool, &alice, &new).await.unwrap();

        let mine = last_room_for_source(&pool, &alice, ReminderSource::Todo, 7).await;
        let theirs = last_room_for_source(&pool, &bob, ReminderSource::Todo, 7).await;

        assert_eq!(mine.unwrap().as_deref(), Some("!alice:room"));
        assert_eq!(theirs.unwrap(), None);
    }
}
