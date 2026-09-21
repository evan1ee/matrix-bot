use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS todos (
            id {{PK}},
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            text TEXT NOT NULL,
            priority TEXT NOT NULL DEFAULT 'medium',
            due_at TEXT,
            completed BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(owner_id, room_id);"
    )?;

    // How long before `due_at` to fire a reminder. Kept on the todo rather than
    // derived from the linked reminder so the web UI can read the current
    // setting back without reverse-engineering it from a timestamp difference.
    let _ = db_ddl!(
        pool,
        "ALTER TABLE todos ADD COLUMN remind_before_minutes BIGINT;"
    );

    // `due_at` used to hold the rendered display string (`2026-12-31 23:59 UTC`),
    // which cannot be compared or subtracted from. Rewrite legacy rows into the
    // timestamp format sqlx reads back into `DateTime<Utc>`. The `strftime`
    // guard leaves anything unparseable alone rather than nulling the column.
    let _ = db_ddl!(
        pool,
        r#"
        UPDATE todos
        SET due_at = strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', ''))
        WHERE due_at LIKE '% UTC'
          AND strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', '')) IS NOT NULL;
        "#
    );

    // Clean up legacy matrix.org room_id placeholders
    let _ = db_ddl!(
        pool,
        "UPDATE todos SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';"
    );

    Ok(())
}
