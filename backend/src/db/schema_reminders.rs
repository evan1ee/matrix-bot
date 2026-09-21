use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS reminders (
            id {{PK}},
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            message TEXT NOT NULL,
            trigger_at TIMESTAMP NOT NULL,
            recurrence TEXT NOT NULL DEFAULT 'none',
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count BIGINT NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);"
    )?;

    // Migration helper for existing sqlite DBs missing retry_count column
    let _ = db_ddl!(
        pool,
        "ALTER TABLE reminders ADD COLUMN retry_count BIGINT NOT NULL DEFAULT 0;"
    );

    // Links a reminder back to the row whose due date scheduled it, so the
    // reminder can be rescheduled or cancelled when that row changes.
    let _ = db_ddl!(pool, "ALTER TABLE reminders ADD COLUMN todo_id BIGINT;");
    let _ = db_ddl!(pool, "ALTER TABLE reminders ADD COLUMN source_kind TEXT;");
    let _ = db_ddl!(pool, "ALTER TABLE reminders ADD COLUMN source_id BIGINT;");

    // Carry existing todo links over. Guarded on `source_id IS NULL` so a
    // re-run cannot overwrite a link that has since moved.
    let _ = db_ddl!(
        pool,
        "UPDATE reminders SET source_kind = 'todo', source_id = todo_id \
         WHERE todo_id IS NOT NULL AND source_id IS NULL;"
    );

    let _ = db_ddl!(pool, "DROP INDEX IF EXISTS idx_reminders_todo;");

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_reminders_source ON reminders(source_kind, source_id);"
    )?;

    db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_reminders_owner_status_trigger ON reminders(owner_id, status, trigger_at DESC, id DESC);")?;

    db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_reminders_owner_room_status ON reminders(owner_id, room_id, status, trigger_at DESC);")?;

    // Clean up legacy matrix.org room_id placeholders
    let _ = db_ddl!(
        pool,
        "UPDATE reminders SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';"
    );

    Ok(())
}
