use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::modules::todo::models::{TodoItem, TodoPriority};

/// The `TodoItem` column list, in `FromRow` order.
///
/// Every read goes through this so adding a column cannot leave one query
/// silently selecting a stale shape.
const COLUMNS: &str = "id, room_id, owner_id, sender_name, text, priority, \
                       due_at, remind_before_minutes, completed, created_at";

/// Extra `WHERE` predicate narrowing a listing to one completion state.
///
/// `completed` is inlined rather than bound because it is a `bool` the caller
/// already decided, never user text — this keeps the bind order of the callers'
/// remaining parameters (owner, limit, offset) fixed.
fn completed_clause(completed: Option<bool>) -> &'static str {
    match completed {
        Some(true) => " AND completed = TRUE",
        Some(false) => " AND completed = FALSE",
        None => "",
    }
}

pub struct TodoRepository;

impl TodoRepository {
    /// Re-reads a row after a write, so callers always see what was stored
    /// rather than what they hoped was stored.
    async fn fetch(pool: &DbPool, id: i64) -> Result<Option<TodoItem>> {
        let item = crate::db_fetch_optional!(
            pool,
            TodoItem,
            &format!("SELECT {COLUMNS} FROM todos WHERE id = ?"),
            id
        )?;

        Ok(item)
    }

    /// Looks up a single task by ID, without an ownership predicate.
    ///
    /// Callers that act on behalf of a user must check ownership themselves —
    /// see `find_owned`.
    pub async fn find(pool: &DbPool, id: i64) -> Result<Option<TodoItem>> {
        Self::fetch(pool, id).await
    }

    /// Looks up a task that `owner_id` owns, for handlers acting on a caller's
    /// behalf. `owner_id` is the sole ownership key.
    pub async fn find_owned(pool: &DbPool, id: i64, owner_id: &str) -> Result<Option<TodoItem>> {
        let item = crate::db_fetch_optional!(
            pool,
            TodoItem,
            &format!("SELECT {COLUMNS} FROM todos WHERE id = ? AND owner_id = ?"),
            id,
            owner_id
        )?;

        Ok(item)
    }

    // One parameter per column, which is what an INSERT is. Clippy's threshold is 7
    // and this is a row; collapsing them into a struct would spell the row type a
    // second time, minus the fields the database fills in, and every caller would
    // build that struct inline at the call site instead. No clarity is gained.
    #[allow(clippy::too_many_arguments)]
    pub async fn add(
        pool: &DbPool,
        room_id: &str,
        owner_id: &str,
        sender_name: &str,
        text: &str,
        priority: TodoPriority,
        due_at: Option<DateTime<Utc>>,
        remind_before_minutes: Option<i64>,
    ) -> Result<TodoItem> {
        let res = crate::db_scalar_one!(
            pool,
            i64,
            r#"
            INSERT INTO todos (room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed)
            VALUES (?, ?, ?, ?, ?, ?, ?, FALSE)
            RETURNING id
            "#,
            room_id,
            owner_id,
            sender_name,
            text,
            priority.as_str(),
            due_at,
            remind_before_minutes
        )?;

        let id = res;

        Self::fetch(pool, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("inserted todo #{id} vanished before it could be read"))
    }

    pub async fn count_room_active(pool: &DbPool, room_id: &str) -> Result<i64> {
        let count = crate::db_scalar_one!(
            pool,
            i64,
            "SELECT COUNT(*) FROM todos WHERE room_id = ? AND completed = FALSE",
            room_id
        )?;

        Ok(count)
    }

    pub async fn list_room(
        pool: &DbPool,
        room_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TodoItem>> {
        let items = crate::db_fetch_all!(pool, TodoItem, &format!(
            "SELECT {COLUMNS} FROM todos WHERE room_id = ? ORDER BY completed ASC, id ASC LIMIT ? OFFSET ?"
        ), room_id, limit, offset)?;

        Ok(items)
    }

    /// Lists every task owned by `owner_id` across all rooms.
    ///
    /// `owner_id` is the sole ownership key. `sender_name` is a display field
    /// that users control and that collides across identity spaces, so it must
    /// never appear in this predicate.
    pub async fn list_all_user(
        pool: &DbPool,
        owner_id: &str,
        completed: Option<bool>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TodoItem>> {
        let filter = completed_clause(completed);
        let items = crate::db_fetch_all!(
            pool,
            TodoItem,
            &format!(
                "SELECT {COLUMNS} FROM todos WHERE owner_id = ?{filter} \
             ORDER BY completed ASC, id ASC LIMIT ? OFFSET ?"
            ),
            owner_id,
            limit,
            offset
        )?;

        Ok(items)
    }

    /// Lists a single room's tasks owned by `owner_id`.
    pub async fn list_room_user(
        pool: &DbPool,
        room_id: &str,
        owner_id: &str,
        completed: Option<bool>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TodoItem>> {
        let filter = completed_clause(completed);
        let items = crate::db_fetch_all!(
            pool,
            TodoItem,
            &format!(
                "SELECT {COLUMNS} FROM todos WHERE room_id = ? AND owner_id = ?{filter} \
             ORDER BY completed ASC, id DESC LIMIT ? OFFSET ?"
            ),
            room_id,
            owner_id,
            limit,
            offset
        )?;

        Ok(items)
    }

    pub async fn count_room_user(
        pool: &DbPool,
        room_id: &str,
        owner_id: &str,
        completed: Option<bool>,
    ) -> Result<i64> {
        let filter = completed_clause(completed);
        let count = crate::db_scalar_one!(
            pool,
            i64,
            &format!("SELECT COUNT(*) FROM todos WHERE room_id = ? AND owner_id = ?{filter}"),
            room_id,
            owner_id
        )?;

        Ok(count)
    }

    /// Tasks owned by `owner_id` matching `completed`, for pagination metadata.
    pub async fn count_all_user(
        pool: &DbPool,
        owner_id: &str,
        completed: Option<bool>,
    ) -> Result<i64> {
        let filter = completed_clause(completed);
        let count = crate::db_scalar_one!(
            pool,
            i64,
            &format!("SELECT COUNT(*) FROM todos WHERE owner_id = ?{filter}"),
            owner_id
        )?;

        Ok(count)
    }

    pub async fn mark_done(pool: &DbPool, id: i64) -> Result<Option<TodoItem>> {
        Self::set_completed(pool, id, true).await
    }

    /// Sets completion in either direction. The API exposes `completed` as a
    /// boolean, so un-completing has to be as reachable as completing.
    pub async fn set_completed(
        pool: &DbPool,
        id: i64,
        completed: bool,
    ) -> Result<Option<TodoItem>> {
        let result = crate::db_execute!(
            pool,
            "UPDATE todos SET completed = ? WHERE id = ?",
            completed,
            id
        )?;

        if result == 0 {
            return Ok(None);
        }
        Self::fetch(pool, id).await
    }

    pub async fn edit_text(pool: &DbPool, id: i64, new_text: &str) -> Result<Option<TodoItem>> {
        let result =
            crate::db_execute!(pool, "UPDATE todos SET text = ? WHERE id = ?", new_text, id)?;

        if result == 0 {
            return Ok(None);
        }
        Self::fetch(pool, id).await
    }

    pub async fn set_priority(
        pool: &DbPool,
        id: i64,
        priority: TodoPriority,
    ) -> Result<Option<TodoItem>> {
        let result = crate::db_execute!(
            pool,
            "UPDATE todos SET priority = ? WHERE id = ?",
            priority.as_str(),
            id
        )?;

        if result == 0 {
            return Ok(None);
        }
        Self::fetch(pool, id).await
    }

    /// Sets or clears the due date. `None` clears it, which also tears down any
    /// reminder scheduled from it — see `todo::due_reminder::sync`.
    pub async fn set_due_date(
        pool: &DbPool,
        id: i64,
        due_at: Option<DateTime<Utc>>,
    ) -> Result<Option<TodoItem>> {
        let result =
            crate::db_execute!(pool, "UPDATE todos SET due_at = ? WHERE id = ?", due_at, id)?;

        if result == 0 {
            return Ok(None);
        }
        Self::fetch(pool, id).await
    }

    /// Sets or clears how long before the due date to fire a reminder.
    pub async fn set_remind_before(
        pool: &DbPool,
        id: i64,
        minutes: Option<i64>,
    ) -> Result<Option<TodoItem>> {
        let result = crate::db_execute!(
            pool,
            "UPDATE todos SET remind_before_minutes = ? WHERE id = ?",
            minutes,
            id
        )?;

        if result == 0 {
            return Ok(None);
        }
        Self::fetch(pool, id).await
    }

    pub async fn delete(pool: &DbPool, id: i64) -> Result<bool> {
        let result = crate::db_execute!(pool, "DELETE FROM todos WHERE id = ?", id)?;

        Ok(result > 0)
    }

    /// Rows `clear_completed_user` is about to remove, read first so the
    /// caller can announce each one individually.
    pub async fn list_completed_user(pool: &DbPool, owner_id: &str) -> Result<Vec<TodoItem>> {
        let items = crate::db_fetch_all!(
            pool,
            TodoItem,
            &format!("SELECT {COLUMNS} FROM todos WHERE owner_id = ? AND completed = TRUE"),
            owner_id
        )?;

        Ok(items)
    }

    /// Rows `clear_completed_room` is about to remove, read first so the
    /// caller can announce each one individually.
    pub async fn list_completed_room(pool: &DbPool, room_id: &str) -> Result<Vec<TodoItem>> {
        let items = crate::db_fetch_all!(
            pool,
            TodoItem,
            &format!("SELECT {COLUMNS} FROM todos WHERE room_id = ? AND completed = TRUE"),
            room_id
        )?;

        Ok(items)
    }

    pub async fn clear_completed_user(pool: &DbPool, owner_id: &str) -> Result<u64> {
        let result = crate::db_execute!(
            pool,
            "DELETE FROM todos WHERE owner_id = ? AND completed = TRUE",
            owner_id
        )?;

        Ok(result)
    }

    pub async fn clear_completed_room(pool: &DbPool, room_id: &str) -> Result<u64> {
        let result = crate::db_execute!(
            pool,
            "DELETE FROM todos WHERE room_id = ? AND completed = TRUE",
            room_id
        )?;

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use chrono::Duration;

    async fn setup_test_db() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_todo_crud_flow() {
        let pool = setup_test_db().await;

        let item = TodoRepository::add(
            &pool,
            "!room:a",
            "@alice:a",
            "alice",
            "Buy milk",
            TodoPriority::High,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(item.text, "Buy milk");
        assert_eq!(item.priority, "high");
        assert!(!item.completed);
        assert!(item.due_at.is_none());

        let list = TodoRepository::list_all_user(&pool, "@alice:a", None, 25, 0)
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let updated = TodoRepository::edit_text(&pool, item.id, "Buy oat milk")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.text, "Buy oat milk");

        let due = Utc::now() + Duration::days(3);
        let due_item = TodoRepository::set_due_date(&pool, item.id, Some(due))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(due_item.due_at.unwrap().timestamp(), due.timestamp());

        let cleared = TodoRepository::set_due_date(&pool, item.id, None)
            .await
            .unwrap()
            .unwrap();
        assert!(cleared.due_at.is_none());

        let done = TodoRepository::mark_done(&pool, item.id)
            .await
            .unwrap()
            .unwrap();
        assert!(done.completed);

        let cleared = TodoRepository::clear_completed_user(&pool, "@alice:a")
            .await
            .unwrap();
        assert_eq!(cleared, 1);

        let empty_list = TodoRepository::list_all_user(&pool, "@alice:a", None, 25, 0)
            .await
            .unwrap();
        assert!(empty_list.is_empty());
    }

    /// The due date must survive a write/read round trip as an instant, not as
    /// a rendered string — the lead-time reminder is computed by subtracting
    /// from it.
    #[tokio::test]
    async fn test_due_date_round_trips_as_a_timestamp() {
        let pool = setup_test_db().await;
        let due = Utc::now() + Duration::hours(30);

        let item = TodoRepository::add(
            &pool,
            "web",
            "@web-alice:api",
            "alice",
            "Ship it",
            TodoPriority::Medium,
            Some(due),
            Some(60),
        )
        .await
        .unwrap();

        assert_eq!(item.due_at.unwrap().timestamp(), due.timestamp());
        assert_eq!(item.remind_before_minutes, Some(60));

        let reread = TodoRepository::find(&pool, item.id).await.unwrap().unwrap();
        assert_eq!(reread.due_at.unwrap().timestamp(), due.timestamp());
    }

    /// Legacy rows stored the rendered display string. The migration must turn
    /// those into timestamps rather than dropping or nulling them.
    #[tokio::test]
    async fn test_migration_converts_legacy_display_format_due_dates() {
        let pool = setup_test_db().await;

        let item = TodoRepository::add(
            &pool,
            "!room:a",
            "@alice:a",
            "alice",
            "Old task",
            TodoPriority::Low,
            None,
            None,
        )
        .await
        .unwrap();

        // Write the pre-migration shape back in, then re-run migrations.
        crate::db_execute!(
            &pool,
            "UPDATE todos SET due_at = '2026-12-31 23:59 UTC' WHERE id = ?",
            item.id
        )
        .unwrap();

        run_migrations(&pool).await.unwrap();

        let migrated = TodoRepository::find(&pool, item.id).await.unwrap().unwrap();
        let due = migrated
            .due_at
            .expect("legacy due date must survive migration");
        assert_eq!(due.format("%Y-%m-%d %H:%M").to_string(), "2026-12-31 23:59");
    }

    #[tokio::test]
    async fn test_find_owned_enforces_ownership() {
        let pool = setup_test_db().await;

        let item = TodoRepository::add(
            &pool,
            "!room:a",
            "@alice:a",
            "alice",
            "Alice task",
            TodoPriority::Medium,
            None,
            None,
        )
        .await
        .unwrap();

        assert!(TodoRepository::find_owned(&pool, item.id, "@alice:a")
            .await
            .unwrap()
            .is_some());
        assert!(TodoRepository::find_owned(&pool, item.id, "@bob:b")
            .await
            .unwrap()
            .is_none());
    }
}
