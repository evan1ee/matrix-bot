//! PostgreSQL repository for todo checklist steps.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::todos::steps::TodoStep;

struct StepRow {
    id: i64,
    todo_id: i64,
    position: i32,
    text: String,
    done: bool,
}

impl From<StepRow> for TodoStep {
    fn from(r: StepRow) -> Self {
        TodoStep {
            id: r.id,
            todo_id: r.todo_id,
            position: r.position,
            text: r.text,
            done: r.done,
        }
    }
}

/// Steps of `todo_id` in display order.
pub async fn list<'e>(
    conn: impl PgExecutor<'e>,
    todo_id: i64,
) -> Result<Vec<TodoStep>, sqlx::Error> {
    let rows = sqlx::query_as!(
        StepRow,
        r#"SELECT id, todo_id, position, text, done FROM todo_steps
           WHERE todo_id = $1 ORDER BY position"#,
        todo_id
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count<'e>(conn: impl PgExecutor<'e>, todo_id: i64) -> Result<i64, sqlx::Error> {
    let n = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "n!" FROM todo_steps WHERE todo_id = $1"#,
        todo_id
    )
    .fetch_one(conn)
    .await?;
    Ok(n)
}

/// Appends a step after the current last one. The caller holds the todo's row
/// lock, so concurrent appends cannot pick the same position.
pub async fn insert_last<'e>(
    conn: impl PgExecutor<'e>,
    todo_id: i64,
    owner: &AccountId,
    text: &str,
) -> Result<TodoStep, sqlx::Error> {
    let row = sqlx::query_as!(
        StepRow,
        r#"
        INSERT INTO todo_steps (todo_id, owner_id, position, text, done)
        SELECT $1, $2, COALESCE(MAX(position), 0) + 1, $3, FALSE
        FROM todo_steps WHERE todo_id = $1
        RETURNING id, todo_id, position, text, done
        "#,
        todo_id,
        owner.as_uuid(),
        text
    )
    .fetch_one(conn)
    .await?;
    Ok(row.into())
}

/// Updates the given fields of a step of `todo_id`; `None` when the step does
/// not belong to that todo.
pub async fn update<'e>(
    conn: impl PgExecutor<'e>,
    todo_id: i64,
    step_id: i64,
    text: Option<&str>,
    done: Option<bool>,
) -> Result<Option<TodoStep>, sqlx::Error> {
    let row = sqlx::query_as!(
        StepRow,
        r#"
        UPDATE todo_steps
        SET text = COALESCE($3, text), done = COALESCE($4, done)
        WHERE todo_id = $1 AND id = $2
        RETURNING id, todo_id, position, text, done
        "#,
        todo_id,
        step_id,
        text,
        done
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn delete<'e>(
    conn: impl PgExecutor<'e>,
    todo_id: i64,
    step_id: i64,
) -> Result<bool, sqlx::Error> {
    let r = sqlx::query!(
        "DELETE FROM todo_steps WHERE todo_id = $1 AND id = $2",
        todo_id,
        step_id
    )
    .execute(conn)
    .await?;
    Ok(r.rows_affected() > 0)
}

/// Closes gaps left by a delete, keeping the existing order.
///
/// Positions are negated first so `UNIQUE (todo_id, position)` holds after
/// every statement while rows move.
pub async fn renumber(conn: &mut sqlx::PgConnection, todo_id: i64) -> Result<(), sqlx::Error> {
    negate_positions(&mut *conn, todo_id).await?;
    sqlx::query!(
        r#"
        UPDATE todo_steps s SET position = o.rn::int
        FROM (SELECT id, row_number() OVER (ORDER BY abs(position)) AS rn
              FROM todo_steps WHERE todo_id = $1) o
        WHERE s.id = o.id
        "#,
        todo_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Reorders the steps to follow `ids`. The caller has checked `ids` is exactly
/// the todo's step set.
pub async fn set_order(
    conn: &mut sqlx::PgConnection,
    todo_id: i64,
    ids: &[i64],
) -> Result<(), sqlx::Error> {
    negate_positions(&mut *conn, todo_id).await?;
    sqlx::query!(
        r#"
        UPDATE todo_steps s SET position = o.rn::int
        FROM (SELECT id, row_number() OVER (ORDER BY array_position($2::bigint[], id)) AS rn
              FROM todo_steps WHERE todo_id = $1) o
        WHERE s.id = o.id
        "#,
        todo_id,
        ids
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn negate_positions(conn: &mut sqlx::PgConnection, todo_id: i64) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE todo_steps SET position = -position WHERE todo_id = $1",
        todo_id
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// `(todo_id, done, total)` for each of `todo_ids` that has steps.
pub async fn progress_for<'e>(
    conn: impl PgExecutor<'e>,
    owner: &AccountId,
    todo_ids: &[i64],
) -> Result<Vec<(i64, i64, i64)>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT todo_id,
               COUNT(*) FILTER (WHERE done) AS "done!",
               COUNT(*) AS "total!"
        FROM todo_steps WHERE todo_id = ANY($1) AND owner_id = $2
        GROUP BY todo_id ORDER BY todo_id
        "#,
        todo_ids,
        owner.as_uuid()
    )
    .fetch_all(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| (r.todo_id, r.done, r.total))
        .collect())
}

/// Copies `from_todo`'s steps onto `to_todo` in the same order, all unchecked.
/// Both todos belong to `owner`; steps of another owner are never copied.
pub async fn copy_unchecked<'e>(
    conn: impl PgExecutor<'e>,
    from_todo: i64,
    to_todo: i64,
    owner: &AccountId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO todo_steps (todo_id, owner_id, position, text, done)
        SELECT $2, owner_id, position, text, FALSE
        FROM todo_steps
        WHERE todo_id = $1 AND owner_id = $3
        ORDER BY position
        "#,
        from_todo,
        to_todo,
        owner.as_uuid()
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::RecurrencePattern;
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::infrastructure::postgres::repositories::todos;

    async fn seeded(seed: &str) -> (crate::db::DbPool, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        (pool, owner)
    }

    async fn todo(pool: &crate::db::DbPool, owner: &AccountId) -> i64 {
        let input = NewTodo {
            room_id: "web".to_string(),
            sender_name: "t".to_string(),
            text: "todo".to_string(),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        };
        todos::create(pool, owner, &input).await.unwrap().id
    }

    async fn texts(pool: &crate::db::DbPool, todo_id: i64) -> Vec<(i32, String)> {
        list(pool, todo_id)
            .await
            .unwrap()
            .into_iter()
            .map(|s| (s.position, s.text))
            .collect()
    }

    /// Given steps 1,2,3, When step 2 is deleted and renumbered, Then positions are 1,2.
    #[tokio::test]
    async fn positions_compact_after_delete() {
        let (pool, owner) = seeded("steps_compact").await;
        let t = todo(&pool, &owner).await;
        let mut ids = vec![];
        for text in ["a", "b", "c"] {
            ids.push(insert_last(&pool, t, &owner, text).await.unwrap().id);
        }
        assert!(delete(&pool, t, ids[1]).await.unwrap());
        renumber(&mut pool.acquire().await.unwrap(), t)
            .await
            .unwrap();
        assert_eq!(
            texts(&pool, t).await,
            vec![(1, "a".to_string()), (2, "c".to_string())]
        );
        assert_eq!(count(&pool, t).await.unwrap(), 2);
    }

    /// Given three steps, When set_order lists them reversed, Then they read back reversed.
    #[tokio::test]
    async fn set_order_reorders() {
        let (pool, owner) = seeded("steps_order").await;
        let t = todo(&pool, &owner).await;
        let mut ids = vec![];
        for text in ["a", "b", "c"] {
            ids.push(insert_last(&pool, t, &owner, text).await.unwrap().id);
        }
        ids.reverse();
        set_order(&mut pool.acquire().await.unwrap(), t, &ids)
            .await
            .unwrap();
        assert_eq!(
            texts(&pool, t).await,
            vec![
                (1, "c".to_string()),
                (2, "b".to_string()),
                (3, "a".to_string())
            ]
        );
    }

    /// Given a todo with steps, When the todo is deleted, Then its steps are gone.
    #[tokio::test]
    async fn steps_cascade_with_todo() {
        let (pool, owner) = seeded("steps_cascade").await;
        let t = todo(&pool, &owner).await;
        insert_last(&pool, t, &owner, "a").await.unwrap();
        todos::delete(&pool, t).await.unwrap();
        assert_eq!(count(&pool, t).await.unwrap(), 0);
    }

    /// Given done steps, When copied, Then the copy is unchecked and in order.
    #[tokio::test]
    async fn copy_unchecked_resets_done_and_keeps_order() {
        let (pool, owner) = seeded("steps_copy").await;
        let from = todo(&pool, &owner).await;
        let to = todo(&pool, &owner).await;
        let a = insert_last(&pool, from, &owner, "a").await.unwrap();
        insert_last(&pool, from, &owner, "b").await.unwrap();
        update(&pool, from, a.id, None, Some(true)).await.unwrap();
        copy_unchecked(&pool, from, to, &owner).await.unwrap();
        let copied = list(&pool, to).await.unwrap();
        assert_eq!(
            copied
                .iter()
                .map(|s| (s.position, s.text.as_str(), s.done))
                .collect::<Vec<_>>(),
            vec![(1, "a", false), (2, "b", false)]
        );
    }

    /// Given steps on two todos, When progress is asked, Then it is (done, total) per todo.
    #[tokio::test]
    async fn progress_counts_done_and_total() {
        let (pool, owner) = seeded("steps_progress").await;
        let t = todo(&pool, &owner).await;
        let other = todo(&pool, &owner).await;
        let a = insert_last(&pool, t, &owner, "a").await.unwrap();
        insert_last(&pool, t, &owner, "b").await.unwrap();
        update(&pool, t, a.id, None, Some(true)).await.unwrap();
        let rows = progress_for(&pool, &owner, &[t, other]).await.unwrap();
        assert_eq!(rows, vec![(t, 1, 2)]);
        let stranger = AccountId::from_seed("steps_progress_other");
        assert!(progress_for(&pool, &stranger, &[t])
            .await
            .unwrap()
            .is_empty());
        // A step id from another todo cannot be updated through this one.
        assert!(update(&pool, other, a.id, Some("x"), None)
            .await
            .unwrap()
            .is_none());
    }
}
