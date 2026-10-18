//! PostgreSQL repository for todo checklist steps.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

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
