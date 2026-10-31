//! PostgreSQL repository for todo items.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::reminders::RecurrencePattern;
use crate::domain::tags::normalize_tags_lenient;
use crate::domain::todos::{NewTodo, TodoItem, TodoPatch, TodoPriority};

#[derive(Debug, sqlx::FromRow)]
struct TodoRow {
    id: i64,
    room_id: String,
    owner_id: uuid::Uuid,
    sender_name: String,
    text: String,
    priority: String,
    due_at: Option<DateTime<Utc>>,
    remind_before_minutes: Option<i64>,
    completed: bool,
    created_at: Option<DateTime<Utc>>,
    tags: Vec<String>,
    recurrence: String,
    recurrence_anchor: Option<DateTime<Utc>>,
    next_todo_id: Option<i64>,
}

impl From<TodoRow> for TodoItem {
    fn from(r: TodoRow) -> Self {
        Self {
            id: r.id,
            room_id: r.room_id,
            owner_id: AccountId::from_uuid(r.owner_id),
            sender_name: r.sender_name,
            text: r.text,
            priority: TodoPriority::from_str_or_default(&r.priority),
            due_at: r.due_at,
            remind_before_minutes: r.remind_before_minutes,
            completed: r.completed,
            created_at: r.created_at,
            tags: normalize_tags_lenient(&r.tags),
            recurrence: RecurrencePattern::from_str_or_default(&r.recurrence),
            recurrence_anchor: r.recurrence_anchor,
            next_todo_id: r.next_todo_id,
        }
    }
}

pub async fn create<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    input: &NewTodo,
) -> Result<TodoItem, sqlx::Error> {
    let priority_str = input.priority.as_str();
    let tags: Vec<String> = input.tags.iter().map(|t| t.as_str().to_string()).collect();

    let row = sqlx::query_as!(
        TodoRow,
        r#"
        INSERT INTO todos (room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed,
                           tags, recurrence, recurrence_anchor)
        VALUES ($1, $2, $3, $4, $5, $6, $7, FALSE, $8, $9, $10)
        RETURNING id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        "#,
        input.room_id,
        owner_id.as_uuid(),
        input.sender_name,
        input.text,
        priority_str,
        input.due_at,
        input.remind_before_minutes,
        &tags,
        input.recurrence.as_str(),
        input.recurrence_anchor
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find<'e>(conn: impl PgExecutor<'e>, id: i64) -> Result<Option<TodoItem>, sqlx::Error> {
    let row = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
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
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        FROM todos
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_all_user<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    completed: Option<bool>,
    tag: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        FROM todos
        WHERE owner_id = $1
          AND ($2::boolean IS NULL OR completed = $2)
          AND ($5::text IS NULL OR tags @> ARRAY[$5::text])
        ORDER BY completed ASC, id ASC
        LIMIT $3 OFFSET $4
        "#,
        owner_id.as_uuid(),
        completed,
        limit,
        offset,
        tag
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

/// Open todos that have a due date, soonest first (the calendar feed's agenda).
pub async fn list_open_dated<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    limit: i64,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        FROM todos
        WHERE owner_id = $1 AND completed = FALSE AND due_at IS NOT NULL
        ORDER BY due_at ASC, id ASC
        LIMIT $2
        "#,
        owner_id.as_uuid(),
        limit
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count_all_user<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    completed: Option<bool>,
    tag: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM todos
        WHERE owner_id = $1
          AND ($2::boolean IS NULL OR completed = $2)
          AND ($3::text IS NULL OR tags @> ARRAY[$3::text])
        "#,
        owner_id.as_uuid(),
        completed,
        tag
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
    tag: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<TodoItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        FROM todos
        WHERE room_id = $1 AND owner_id = $2
          AND ($3::boolean IS NULL OR completed = $3)
          AND ($6::text IS NULL OR tags @> ARRAY[$6::text])
        ORDER BY completed ASC, id DESC
        LIMIT $4 OFFSET $5
        "#,
        room_id,
        owner_id.as_uuid(),
        completed,
        limit,
        offset,
        tag
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
    tag: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM todos
        WHERE room_id = $1 AND owner_id = $2
          AND ($3::boolean IS NULL OR completed = $3)
          AND ($4::text IS NULL OR tags @> ARRAY[$4::text])
        "#,
        room_id,
        owner_id.as_uuid(),
        completed,
        tag
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
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
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
    anchor: Option<Option<DateTime<Utc>>>,
) -> Result<Option<TodoItem>, sqlx::Error> {
    let priority_str = patch.priority.map(|p| p.as_str());
    let has_due_at = patch.due_at.is_some();
    let due_at_val = patch.due_at.flatten();
    let has_remind = patch.remind_before_minutes.is_some();
    let remind_val = patch.remind_before_minutes.flatten();
    let has_tags = patch.tags.is_some();
    let tags_val: Vec<String> = patch
        .tags
        .iter()
        .flatten()
        .map(|t| t.as_str().to_string())
        .collect();
    let recurrence_val = patch.recurrence.map(|r| r.as_str());
    let has_anchor = anchor.is_some();
    let anchor_val = anchor.flatten();

    let row = sqlx::query_as!(
        TodoRow,
        r#"
        UPDATE todos
        SET
            text = COALESCE($1, text),
            priority = COALESCE($2, priority),
            completed = COALESCE($3, completed),
            due_at = CASE WHEN $4 THEN $5 ELSE due_at END,
            remind_before_minutes = CASE WHEN $6 THEN $7 ELSE remind_before_minutes END,
            tags = CASE WHEN $8 THEN $9 ELSE tags END,
            recurrence = COALESCE($10, recurrence),
            recurrence_anchor = CASE WHEN $11 THEN $12 ELSE recurrence_anchor END
        WHERE id = $13 AND owner_id = $14
        RETURNING id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        "#,
        patch.text.as_deref(),
        priority_str,
        patch.completed,
        has_due_at,
        due_at_val,
        has_remind,
        remind_val,
        has_tags,
        &tags_val,
        recurrence_val,
        has_anchor,
        anchor_val,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn lock_owned<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<TodoItem>, sqlx::Error> {
    let row = sqlx::query_as!(
        TodoRow,
        r#"
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        FROM todos
        WHERE id = $1 AND owner_id = $2
        FOR UPDATE
        "#,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn set_next_todo_id<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    next: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query!("UPDATE todos SET next_todo_id = $1 WHERE id = $2", next, id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Distinct tags of the owner's active todos with how many todos carry each.
pub async fn list_tags<'e>(
    conn: impl PgExecutor<'e>,
    owner_id: &AccountId,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT t AS "tag!", COUNT(*) AS "count!"
        FROM todos, unnest(tags) t
        WHERE owner_id = $1 AND NOT completed
        GROUP BY t
        ORDER BY t
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(|r| (r.tag, r.count)).collect())
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
        RETURNING id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
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
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
        FROM todos
        WHERE owner_id = $1 AND completed = TRUE
        "#,
        owner_id.as_uuid()
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
        SELECT id, room_id, owner_id, sender_name, text, priority, due_at, remind_before_minutes, completed, created_at,
               tags, recurrence, recurrence_anchor, next_todo_id
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
        owner_id.as_uuid()
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
    use crate::domain::tags::Tag;

    async fn setup_test_db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_todos_repository_crud() {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed("acct_bob");

        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'bob', 'active')",
            owner.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let input = NewTodo {
            room_id: "!room:todos".to_string(),
            sender_name: "bob".to_string(),
            text: "Implement feature X".to_string(),
            priority: TodoPriority::High,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
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

        let updated = update(&pool, &owner, created.id, &patch, None)
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

        let remaining = count_all_user(&pool, &owner, None, None).await.unwrap();
        assert_eq!(remaining, 0);
    }

    fn tagged(text: &str, tags: &[&str]) -> NewTodo {
        NewTodo {
            room_id: "web".to_string(),
            sender_name: "t".to_string(),
            text: text.to_string(),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: tags.iter().map(|t| Tag::parse(t).unwrap()).collect(),
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        }
    }

    async fn seeded(seed: &str) -> (crate::db::DbPool, AccountId) {
        let pool = setup_test_db().await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        (pool, owner)
    }

    /// Given tags and a recurrence, When the todo is read back, Then they round-trip.
    #[tokio::test]
    async fn tags_and_recurrence_round_trip() {
        let (pool, owner) = seeded("todo_repo_roundtrip").await;
        let due = crate::utils::time::now_at_storage_precision();
        let input = NewTodo {
            due_at: Some(due),
            recurrence: RecurrencePattern::Monthly,
            recurrence_anchor: Some(due),
            ..tagged("pay rent", &["work", "home"])
        };

        let created = create(&pool, &owner, &input).await.unwrap();
        let found = find_owned(&pool, &owner, created.id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(found, created);
        let names: Vec<&str> = found.tags.iter().map(Tag::as_str).collect();
        assert_eq!(names, ["home", "work"]);
        assert_eq!(found.recurrence, RecurrencePattern::Monthly);
        assert_eq!(found.recurrence_anchor, Some(due));
        assert_eq!(found.next_todo_id, None);
    }

    /// Given two owners with a work tag, When owner A filters, Then only A's rows match.
    #[tokio::test]
    async fn tag_filter_is_owner_isolated() {
        let (pool, a) = seeded("todo_repo_tag_a").await;
        let b = AccountId::from_seed("todo_repo_tag_b");
        crate::db::pool::seed_account(&pool, &b).await;
        create(&pool, &a, &tagged("a task", &["work"]))
            .await
            .unwrap();
        create(&pool, &a, &tagged("a other", &["home"]))
            .await
            .unwrap();
        create(&pool, &b, &tagged("b task", &["work"]))
            .await
            .unwrap();

        let listed = list_all_user(&pool, &a, None, Some("work"), 50, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].text, "a task");
        assert_eq!(
            count_all_user(&pool, &a, None, Some("work")).await.unwrap(),
            1
        );
        assert_eq!(count_all_user(&pool, &a, None, None).await.unwrap(), 2);

        let in_room = list_room_user(&pool, "web", &a, None, Some("work"), 50, 0)
            .await
            .unwrap();
        assert_eq!(in_room.len(), 1);
        assert_eq!(
            count_room_user(&pool, "web", &a, None, Some("work"))
                .await
                .unwrap(),
            1
        );
    }

    /// Given completed and active todos, When tags are listed, Then only active ones count.
    #[tokio::test]
    async fn list_tags_counts_active_only() {
        let (pool, owner) = seeded("todo_repo_list_tags").await;
        create(&pool, &owner, &tagged("1", &["work"]))
            .await
            .unwrap();
        create(&pool, &owner, &tagged("2", &["work"]))
            .await
            .unwrap();
        let done = create(&pool, &owner, &tagged("3", &["work"]))
            .await
            .unwrap();
        set_completed(&pool, done.id, true).await.unwrap();
        create(&pool, &owner, &tagged("4", &["home"]))
            .await
            .unwrap();

        assert_eq!(
            list_tags(&pool, &owner).await.unwrap(),
            vec![("home".to_string(), 1), ("work".to_string(), 2)]
        );
    }

    /// Given a direct insert bypassing validation, When recurrence has no due date,
    /// Then the database CHECK refuses it.
    #[tokio::test]
    async fn db_check_refuses_recurrence_without_due() {
        let (pool, owner) = seeded("todo_repo_check").await;
        let result = sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, recurrence)
             VALUES ('web', $1, 't', 'x', 'low', 'daily')",
        )
        .bind(owner.as_uuid())
        .execute(&pool)
        .await;
        assert!(result.is_err());
    }

    /// Given a todo and its successor, When the link is set and the row locked, Then both read back.
    #[tokio::test]
    async fn lock_owned_and_set_next_todo_id() {
        let (pool, owner) = seeded("todo_repo_lock").await;
        let first = create(&pool, &owner, &tagged("a", &[])).await.unwrap();
        let second = create(&pool, &owner, &tagged("b", &[])).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        let locked = lock_owned(&mut *tx, &owner, first.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(locked.id, first.id);
        set_next_todo_id(&mut *tx, first.id, second.id)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        let other = AccountId::from_seed("todo_repo_lock_other");
        let mut tx = pool.begin().await.unwrap();
        assert!(lock_owned(&mut *tx, &other, first.id)
            .await
            .unwrap()
            .is_none());
        drop(tx);
        let found = find_owned(&pool, &owner, first.id).await.unwrap().unwrap();
        assert_eq!(found.next_todo_id, Some(second.id));
    }
}
