//! PostgreSQL repository for Matrix bot message targets linked to domain entities (todos, reminders).

use crate::domain::accounts::AccountId;
use crate::domain::chat::BotMessageTarget;

struct BotMessageTargetRow {
    event_id: String,
    room_id: String,
    entity_type: String,
    entity_id: i64,
    owner_id: uuid::Uuid,
    title: String,
}

impl From<BotMessageTargetRow> for BotMessageTarget {
    fn from(r: BotMessageTargetRow) -> Self {
        Self {
            event_id: r.event_id,
            room_id: r.room_id,
            entity_type: r.entity_type,
            entity_id: r.entity_id,
            owner_id: AccountId::from_uuid(r.owner_id),
            title: r.title,
        }
    }
}

pub async fn record_target(
    conn: impl sqlx::PgExecutor<'_>,
    event_id: &str,
    room_id: &str,
    entity_type: &str,
    entity_id: i64,
    owner_id: &AccountId,
    title: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT(event_id) DO UPDATE SET
            room_id = excluded.room_id,
            entity_type = excluded.entity_type,
            entity_id = excluded.entity_id,
            owner_id = excluded.owner_id,
            title = excluded.title
        "#,
        event_id,
        room_id,
        entity_type,
        entity_id,
        owner_id.as_uuid(),
        title
    )
    .execute(conn)
    .await?;

    Ok(())
}

pub async fn get_target(
    conn: impl sqlx::PgExecutor<'_>,
    event_id: &str,
) -> Result<Option<BotMessageTarget>, sqlx::Error> {
    let row = sqlx::query_as!(
        BotMessageTargetRow,
        r#"
        SELECT event_id, room_id, entity_type, entity_id, owner_id, title
        FROM bot_message_targets
        WHERE event_id = $1
        "#,
        event_id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete_target_by_entity(
    conn: impl sqlx::PgExecutor<'_>,
    entity_type: &str,
    entity_id: i64,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM bot_message_targets WHERE entity_type = $1 AND entity_id = $2",
        entity_type,
        entity_id
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected())
}

pub async fn copy_targets(
    conn: impl sqlx::PgExecutor<'_>,
    from_event_id: &str,
    to_event_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        INSERT INTO bot_message_targets (event_id, room_id, entity_type, entity_id, owner_id, title)
        SELECT $1, room_id, entity_type, entity_id, owner_id, title
        FROM bot_message_targets
        WHERE event_id = $2
        ON CONFLICT(event_id) DO UPDATE SET
            room_id = excluded.room_id,
            entity_type = excluded.entity_type,
            entity_id = excluded.entity_id,
            owner_id = excluded.owner_id,
            title = excluded.title
        "#,
        to_event_id,
        from_event_id
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_bot_message_targets_crud() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let owner_id =
            crate::infrastructure::postgres::repositories::identities::resolve_or_provision(
                &pool,
                &crate::domain::channels::PlatformIdentity::matrix("@target_user:mbm.one").unwrap(),
            )
            .await
            .unwrap();

        record_target(
            &pool,
            "$ev_1",
            "!room:a",
            "todo",
            42,
            &owner_id,
            "Test task",
        )
        .await
        .unwrap();

        let target = get_target(&pool, "$ev_1").await.unwrap().unwrap();
        assert_eq!(target.event_id, "$ev_1");
        assert_eq!(target.room_id, "!room:a");
        assert_eq!(target.entity_type, "todo");
        assert_eq!(target.entity_id, 42);
        assert_eq!(target.title, "Test task");

        // Copy targets
        let copied = copy_targets(&pool, "$ev_1", "$ev_2").await.unwrap();
        assert_eq!(copied, 1);
        let target2 = get_target(&pool, "$ev_2").await.unwrap().unwrap();
        assert_eq!(target2.event_id, "$ev_2");
        assert_eq!(target2.title, "Test task");

        // Delete by entity
        let deleted = delete_target_by_entity(&pool, "todo", 42).await.unwrap();
        assert_eq!(deleted, 2);

        assert!(get_target(&pool, "$ev_1").await.unwrap().is_none());
        assert!(get_target(&pool, "$ev_2").await.unwrap().is_none());
    }
}
