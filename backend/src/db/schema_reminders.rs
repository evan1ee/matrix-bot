use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            message TEXT NOT NULL,
            trigger_at TIMESTAMP NOT NULL,
            recurrence TEXT NOT NULL DEFAULT 'none',
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);",
    )
    .execute(pool)
    .await?;

    // Migration helper for existing sqlite DBs missing retry_count column
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;")
        .execute(pool)
        .await;

    // Links a reminder back to the row whose due date scheduled it, so the
    // reminder can be rescheduled or cancelled when that row changes.
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN todo_id INTEGER;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN source_kind TEXT;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN source_id INTEGER;")
        .execute(pool)
        .await;

    // Carry existing todo links over. Guarded on `source_id IS NULL` so a
    // re-run cannot overwrite a link that has since moved.
    let _ = sqlx::query(
        "UPDATE reminders SET source_kind = 'todo', source_id = todo_id \
         WHERE todo_id IS NOT NULL AND source_id IS NULL;",
    )
    .execute(pool)
    .await;

    let _ = sqlx::query("DROP INDEX IF EXISTS idx_reminders_todo;")
        .execute(pool)
        .await;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_source ON reminders(source_kind, source_id);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_owner_status_trigger ON reminders(owner_id, status, trigger_at DESC, id DESC);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_owner_room_status ON reminders(owner_id, room_id, status, trigger_at DESC);",
    )
    .execute(pool)
    .await?;

    // Clean up legacy matrix.org room_id placeholders
    let _ = sqlx::query("UPDATE reminders SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;

    Ok(())
}
