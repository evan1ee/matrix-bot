//! Reminder lookups keyed by the entity a reminder was derived from, kept
//! apart from `reminders.rs` to hold that file under the line cap.

use sqlx::PgExecutor;

use crate::domain::reminders::ReminderSource;

/// Room of the most recent reminder of any status for `source`/`source_id`.
pub async fn last_room_for_source<'e>(
    conn: impl PgExecutor<'e>,
    source: ReminderSource,
    source_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar!(
        r#"
        SELECT room_id FROM reminders
        WHERE source_kind = $1 AND source_id = $2
        ORDER BY id DESC
        LIMIT 1
        "#,
        source.as_str(),
        source_id
    )
    .fetch_optional(conn)
    .await
}
