//! PostgreSQL repository for todo items.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::todos::{NewTodo, TodoItem, TodoPatch, TodoPriority};

#[derive(Debug, sqlx::FromRow)]
struct TodoRow {
    id: i64,
    room_id: String,
    owner_id: String,
    sender_name: String,
    text: String,
    priority: String,
    due_at: Option<DateTime<Utc>>,
    remind_before_minutes: Option<i64>,
    completed: bool,
    created_at: Option<DateTime<Utc>>,
}

impl From<TodoRow> for TodoItem {
    fn from(r: TodoRow) -> Self {
        Self {
            id: r.id,
            room_id: r.room_id,
            owner_id: AccountId::new(r.owner_id),
            sender_name: r.sender_name,
            text: r.text,
            priority: TodoPriority::from_str_or_default(&r.priority),
            due_at: r.due_at,
            remind_before_minutes: r.remind_before_minutes,
            completed: r.completed,
            created_at: r.created_at,
        }
    }
}

pub async fn create<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    input: &NewTodo,
) -> Result<TodoItem, sqlx::Error> {
    let priority_str = input.priority.as_str();

    let row = sqlx::query_as!(
        TodoRow,
        r#"
        INSERT INTO todos (room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed)
        VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE)
        RETURNING id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        "#,
        input.room_id,
        owner_id.as_str(),
        input.sender_name,
        input.text,
        priority_str,
        input.due_at,
        input.remind_before_minutes
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find<'e>(conn: impl PgExecutor<'e>, id: i64) -> Result<Option<TodoItem>, sqlx::Error> {
    let row = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE id = $1
        "#,
        id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_owned<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<TodoItem>, sqlx::Error> {
    let row = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_all_user<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    completed: Option<bool>,
    limit: i64,
    offset: i64,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE owner_id = $1
          AND ($2::boolean IS NULL OR completed = $2)
        ORDER BY completed ASC, id ASC
        LIMIT $3 OFFSET $4
        "#,
        owner_id.as_str(),
        completed,
        limit,
        offset
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count_all_user<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    completed: Option<bool>,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM todos
        WHERE owner_id = $1
          AND ($2::boolean IS NULL OR completed = $2)
        "#,
        owner_id.as_str(),
        completed
    )
    .fetch_one(conn)
    .await?;

    Ok(count.unwrap_or(0))
}

pub async fn list_room_user<'e>(
    conn: impl PgExecutor<'e>,
    room_id: &str,
    owner_id: &AccountId,
    completed: Option<bool>,
    limit: i64,
    offset: i64,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE room_id = $1 AND owner_id = $2
          AND ($3::boolean IS NULL OR completed = $3)
        ORDER BY completed ASC, id DESC
        LIMIT $4 OFFSET $5
        "#,
        room_id,
        owner_id.as_str(),
        completed,
        limit,
        offset
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count_room_user<'e>(
    conn: impl PgExecutor<'e>,
    room_id: &str,
    owner_id: &AccountId,
    completed: Option<bool>,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM todos
        WHERE room_id = $1 AND owner_id = $2
          AND ($3::boolean IS NULL OR completed = $3)
        "#,
        room_id,
        owner_id.as_str(),
        completed
    )
    .fetch_one(conn)
    .await?;

    Ok(count.unwrap_or(0))
}

pub async fn list_room<'e>(
    conn: impl PgExecutor<'e>,
    room_id: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE room_id = $1
        ORDER BY completed ASC, id ASC
        LIMIT $2 OFFSET $3
        "#,
        room_id,
        limit,
        offset
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count_room_active<'e>(
    conn: impl PgExecutor<'e>,
    room_id: &str,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) FROM todos WHERE room_id = $1 AND completed = FALSE
        "#,
        room_id
    )
    .fetch_one(conn)
    .await?;

    Ok(count.unwrap_or(0))
}

pub async fn update<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
    patch: &TodoPatch,
) -> Result<Option<TodoItem>, sqlx::Error> {
    let priority_str = patch.priority.map(|p| p.as_str());
    let has_due_at = patch.due_at.is_some();
    let due_at_val = patch.due_at.flatten();
    let has_remind = patch.remind_before_minutes.is_some();
    let remind_val = patch.remind_before_minutes.flatten();

    let row = sqlx::query_as!(
        TodoRow,
        r#"
        UPDATE todos
        SET
            text = COALESCE($1, text),
            priority = COALESCE($2, priority),
            completed = COALESCE($3, completed),
            due_at = CASE WHEN $4 THEN $5 ELSE due_at END,
            remind_before_minutes = CASE WHEN $6 THEN $7 ELSE remind_before_minutes END
        WHERE id = $8 AND owner_id = $9
        RETURNING id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        "#,
        patch.text.as_deref(),
        priority_str,
        patch.completed,
        has_due_at,
        due_at_val,
        has_remind,
        remind_val,
        id,
        owner_id.as_str()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn set_completed<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    completed: bool,
) -> Result<Option<TodoItem>, sqlx::Error> {
    let row = sqlx::query_as!(
        TodoRow,
        r#"
        UPDATE todos
        SET completed = $1
        WHERE id = $2
        RETURNING id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        "#,
        completed,
        id
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete<'e>(conn: impl PgExecutor<'e>, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(r#"DELETE FROM todos WHERE id = $1"#, id)
        .execute(conn)
        .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn list_completed_user<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE owner_id = $1 AND completed = TRUE
        "#,
        owner_id.as_str()
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn list_completed_room<'e>(
    conn: impl PgExecutor<'e>,
    room_id: &str,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at
        FROM todos
        WHERE room_id = $1 AND completed = TRUE
        "#,
        room_id
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn clear_completed_user<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM todos WHERE owner_id = $1 AND completed = TRUE"#,
        owner_id.as_str()
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected())
}

pub async fn clear_completed_room<'e>(
    conn: impl PgExecutor<'e>,
    room_id: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query!(
        r#"DELETE FROM todos WHERE room_id = $1 AND completed = TRUE"#,
        room_id
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_todos_repository_crud() {
        let pool = setup_test_db().await;
        let owner = AccountId::new("acct_bob");

        let input = NewTodo {
            room_id: "!room:todos".to_string(),
            sender_name: "bob".to_string(),
            text: "Implement feature X".to_string(),
            priority: TodoPriority::High,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
        };

        let created = create(&pool, &owner, &input).await.unwrap();
        assert_eq!(created.text, "Implement feature X");
        assert_eq!(created.priority, TodoPriority::High);
        assert!(!created.completed);

        let found = find_owned(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, created.id);

        let patch = TodoPatch {
            text: Some("Implement feature X and Y".to_string()),
            priority: Some(TodoPriority::Medium),
            ..Default::default()
        };

        let updated = update(&pool, &owner, created.id, &patch)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(updated.text, "Implement feature X and Y");
        assert_eq!(updated.priority, TodoPriority::Medium);

        let done = set_completed(&pool, created.id, true)
            .await
            .unwrap()
            .unwrap();
        assert!(done.completed);

        let completed_list = list_completed_user(&pool, &owner).await.unwrap();
        assert_eq!(completed_list.len(), 1);

        let cleared = clear_completed_user(&pool, &owner).await.unwrap();
        assert_eq!(cleared, 1);

        let remaining = count_all_user(&pool, &owner, None).await.unwrap();
        assert_eq!(remaining, 0);
    }
}
