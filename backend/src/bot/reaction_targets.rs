//! Repository for tracking Matrix message targets (todos, reminders)
//! that can be interacted with via reactions.

use sqlx::SqlitePool;

/// A tracked message target linked to a domain entity.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct BotMessageTarget {
    pub event_id: String,
    pub room_id: String,
    pub entity_type: String, // "todo" or "reminder"
    pub entity_id: i64,
    pub owner_id: String,
    pub title: String,
}

/// Records or updates a Matrix message target associated with an entity.
pub async fn record_target(
    pool: &SqlitePool,
    event_id: &str,
    room_id: &str,
    entity_type: &str,
    entity_id: i64,
    owner_id: &str,
    title: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title)
        VALUES (?, ?, ?, ?, ?, ?)
        ON CONFLICT(event_id) DO UPDATE SET
            room_id = excluded.room_id,
            entity_type = excluded.entity_type,
            entity_id = excluded.entity_id,
            owner_id = excluded.owner_id,
            title = excluded.title
        "#,
    )
    .bind(event_id)
    .bind(room_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(owner_id)
    .bind(title)
    .execute(pool)
    .await?;

    Ok(())
}

/// Retrieves a tracked message target by Matrix event ID.
pub async fn get_target(
    pool: &SqlitePool,
    event_id: &str,
) -> Result<Option<BotMessageTarget>, sqlx::Error> {
    sqlx::query_as::<_, BotMessageTarget>(
        r#"
        SELECT event_id, room_id, entity_type, entity_id, owner_id, title
        FROM bot_message_targets
        WHERE event_id = ?
        "#,
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await
}

/// Deletes tracked targets for a given entity type and ID.
pub async fn delete_target_by_entity(
    pool: &SqlitePool,
    entity_type: &str,
    entity_id: i64,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query(
        "DELETE FROM bot_message_targets WHERE entity_type = ? AND entity_id = ?",
    )
    .bind(entity_type)
    .bind(entity_id)
    .execute(pool)
    .await?;

    Ok(res.rows_affected())
}

/// Parses entity type, ID, and title from a bot confirmation HTML reply.
pub fn parse_entity_from_reply(reply_html: &str) -> Option<(&'static str, i64, String)> {
    // 1. Todo: "Added Task <b>#(\d+)</b> for .*: <i>\"(.*?)\"</i>"
    if let Some(pos) = reply_html.find("Added Task <b>#") {
        let after_hash = &reply_html[pos + "Added Task <b>#".len()..];
        if let Some(end_b) = after_hash.find("</b>") {
            if let Ok(id) = after_hash[..end_b].trim().parse::<i64>() {
                let title = extract_title_between_quotes(after_hash);
                return Some(("todo", id, title));
            }
        }
    }

    // 2. Reminder: "Reminder #(\d+) set!.*for: <i>\"(.*?)\"</i>"
    if let Some(pos) = reply_html.find("Reminder #") {
        let after_hash = &reply_html[pos + "Reminder #".len()..];
        if let Some(end_b) = after_hash.find(" set!</b>") {
            if let Ok(id) = after_hash[..end_b].trim().parse::<i64>() {
                let title = extract_title_between_quotes(after_hash);
                return Some(("reminder", id, title));
            }
        }
    }

    None
}

fn extract_title_between_quotes(s: &str) -> String {
    if let Some(start) = s.find("<i>\"") {
        let after_start = &s[start + 4..];
        if let Some(end) = after_start.find("\"</i>") {
            return after_start[..end].to_string();
        }
    }
    String::new()
}

/// Attempts to record a target if the reply message indicates an entity was created.
pub async fn try_record_from_reply(
    pool: &SqlitePool,
    event_id: &str,
    room_id: &str,
    owner_id: &str,
    reply_html: &str,
) {
    if let Some((entity_type, entity_id, title)) = parse_entity_from_reply(reply_html) {
        let _ = record_target(
            pool,
            event_id,
            room_id,
            entity_type,
            entity_id,
            owner_id,
            &title,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn setup_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[test]
    fn test_parse_todo_reply() {
        let reply = "✅ Added Task <b>#42</b> for Alice: <i>\"Buy groceries\"</i> [Med]";
        let parsed = parse_entity_from_reply(reply);
        assert_eq!(
            parsed,
            Some(("todo", 42, "Buy groceries".to_string()))
        );
    }

    #[test]
    fn test_parse_reminder_reply() {
        let reply = "⏱️ <b>Reminder #10 set!</b> I will remind you in <b>10 minutes</b> for: <i>\"Call doctor\"</i>";
        let parsed = parse_entity_from_reply(reply);
        assert_eq!(
            parsed,
            Some(("reminder", 10, "Call doctor".to_string()))
        );
    }

    #[tokio::test]
    async fn test_crud_target() {
        let pool = setup_pool().await;

        record_target(
            &pool,
            "$event123",
            "!room:x",
            "todo",
            1,
            "@alice:x",
            "Test task",
        )
        .await
        .unwrap();

        let target = get_target(&pool, "$event123").await.unwrap().unwrap();
        assert_eq!(target.event_id, "$event123");
        assert_eq!(target.room_id, "!room:x");
        assert_eq!(target.entity_type, "todo");
        assert_eq!(target.entity_id, 1);
        assert_eq!(target.owner_id, "@alice:x");
        assert_eq!(target.title, "Test task");

        // Try update via record_target ON CONFLICT
        record_target(
            &pool,
            "$event123",
            "!room:x",
            "todo",
            1,
            "@alice:x",
            "Updated task",
        )
        .await
        .unwrap();

        let target2 = get_target(&pool, "$event123").await.unwrap().unwrap();
        assert_eq!(target2.title, "Updated task");

        // Delete by entity
        let deleted = delete_target_by_entity(&pool, "todo", 1).await.unwrap();
        assert_eq!(deleted, 1);
        assert!(get_target(&pool, "$event123").await.unwrap().is_none());
    }
}
