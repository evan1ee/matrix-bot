use sqlx::SqlitePool;
use anyhow::Result;

use crate::modules::todo::models::{TodoItem, TodoPriority};

pub struct TodoRepository;

impl TodoRepository {
    pub async fn add(
        pool: &SqlitePool,
        room_id: &str,
        sender_mxid: &str,
        sender_name: &str,
        text: &str,
        priority: TodoPriority,
    ) -> Result<TodoItem> {
        let priority_str = priority.as_str();

        let res = sqlx::query(
            r#"
            INSERT INTO todos (room_id, sender_mxid, sender_name, text, priority, completed)
            VALUES (?, ?, ?, ?, ?, 0)
            "#,
        )
        .bind(room_id)
        .bind(sender_mxid)
        .bind(sender_name)
        .bind(text)
        .bind(priority_str)
        .execute(pool)
        .await?;

        let id = res.last_insert_rowid();

        let item = sqlx::query_as::<_, TodoItem>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at
            FROM todos WHERE id = ?
            "#,
        )
        .bind(id)
        .fetch_one(pool)
        .await?;

        Ok(item)
    }

    pub async fn count_room_active(pool: &SqlitePool, room_id: &str) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM todos WHERE room_id = ? AND completed = 0",
        )
        .bind(room_id)
        .fetch_one(pool)
        .await?;

        Ok(count)
    }

    pub async fn list_room(pool: &SqlitePool, room_id: &str, limit: i64, offset: i64) -> Result<Vec<TodoItem>> {
        let items = sqlx::query_as::<_, TodoItem>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at
            FROM todos WHERE room_id = ? ORDER BY completed ASC, id ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(room_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(items)
    }


    /// Lists every task owned by `owner_id` across all rooms.
    ///
    /// `sender_mxid` is the sole ownership key. `sender_name` is a display field
    /// that users control and that collides across identity spaces, so it must
    /// never appear in this predicate.
    pub async fn list_all_user(pool: &SqlitePool, owner_id: &str, limit: i64, offset: i64) -> Result<Vec<TodoItem>> {
        let items = sqlx::query_as::<_, TodoItem>(
            r#"
            SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at
            FROM todos WHERE sender_mxid = ? ORDER BY completed ASC, id ASC
            LIMIT ? OFFSET ?
            "#,
        )
        .bind(owner_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

        Ok(items)
    }

    /// Total tasks owned by `owner_id`, for pagination metadata.
    pub async fn count_all_user(pool: &SqlitePool, owner_id: &str) -> Result<i64> {
        let count = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM todos WHERE sender_mxid = ?",
        )
        .bind(owner_id)
        .fetch_one(pool)
        .await?;

        Ok(count)
    }

    pub async fn mark_done(pool: &SqlitePool, id: i64) -> Result<Option<TodoItem>> {
        Self::set_completed(pool, id, true).await
    }

    /// Sets completion in either direction. The API exposes `completed` as a
    /// boolean, so un-completing has to be as reachable as completing.
    pub async fn set_completed(pool: &SqlitePool, id: i64, completed: bool) -> Result<Option<TodoItem>> {
        let result = sqlx::query(
            "UPDATE todos SET completed = ? WHERE id = ?",
        )
        .bind(completed)
        .bind(id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        let item = sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(item)
    }

    pub async fn edit_text(pool: &SqlitePool, id: i64, new_text: &str) -> Result<Option<TodoItem>> {
        let result = sqlx::query(
            "UPDATE todos SET text = ? WHERE id = ?",
        )
        .bind(new_text)
        .bind(id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        let item = sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(item)
    }

    pub async fn set_priority(pool: &SqlitePool, id: i64, priority: TodoPriority) -> Result<Option<TodoItem>> {
        let priority_str = priority.as_str();
        let result = sqlx::query(
            "UPDATE todos SET priority = ? WHERE id = ?",
        )
        .bind(priority_str)
        .bind(id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        let item = sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(item)
    }

    pub async fn set_due_date(pool: &SqlitePool, id: i64, due_at: &str) -> Result<Option<TodoItem>> {
        let result = sqlx::query(
            "UPDATE todos SET due_at = ? WHERE id = ?",
        )
        .bind(due_at)
        .bind(id)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }

        let item = sqlx::query_as::<_, TodoItem>(
            "SELECT id, room_id, sender_mxid, sender_name, text, priority, due_at, completed, created_at FROM todos WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        Ok(item)
    }

    pub async fn clear_completed_user(pool: &SqlitePool, sender_mxid: &str) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM todos WHERE sender_mxid = ? AND completed = 1",
        )
        .bind(sender_mxid)
        .execute(pool)
        .await?;

        Ok(result.rows_affected())
    }

    pub async fn clear_completed_room(pool: &SqlitePool, room_id: &str) -> Result<u64> {
        let result = sqlx::query(
            "DELETE FROM todos WHERE room_id = ? AND completed = 1",
        )
        .bind(room_id)
        .execute(pool)
        .await?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn setup_test_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_todo_crud_flow() {
        let pool = setup_test_db().await;

        let item = TodoRepository::add(&pool, "!room:a", "@alice:a", "alice", "Buy milk", TodoPriority::High)
            .await
            .unwrap();
        assert_eq!(item.text, "Buy milk");
        assert_eq!(item.priority, "high");
        assert!(!item.completed);

        let list = TodoRepository::list_all_user(&pool, "@alice:a", 25, 0).await.unwrap();
        assert_eq!(list.len(), 1);

        let updated = TodoRepository::edit_text(&pool, item.id, "Buy oat milk").await.unwrap().unwrap();
        assert_eq!(updated.text, "Buy oat milk");

        let due_item = TodoRepository::set_due_date(&pool, item.id, "2026-12-31 23:59 UTC").await.unwrap().unwrap();
        assert_eq!(due_item.due_at.as_deref(), Some("2026-12-31 23:59 UTC"));

        let done = TodoRepository::mark_done(&pool, item.id).await.unwrap().unwrap();
        assert!(done.completed);

        let cleared = TodoRepository::clear_completed_user(&pool, "@alice:a").await.unwrap();
        assert_eq!(cleared, 1);

        let empty_list = TodoRepository::list_all_user(&pool, "@alice:a", 25, 0).await.unwrap();
        assert!(empty_list.is_empty());
    }
}
