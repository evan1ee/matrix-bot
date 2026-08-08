use sqlx::SqlitePool;
use tracing::info;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    info!("Running database table schema initialization...");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            sender_mxid TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            text TEXT NOT NULL,
            priority TEXT NOT NULL DEFAULT 'medium',
            due_at TEXT,
            completed BOOLEAN NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            sender_mxid TEXT NOT NULL,
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
        r#"
        CREATE TABLE IF NOT EXISTS web_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Composite indexes for performance optimization
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(sender_mxid, room_id);"
    )
    .execute(pool)
    .await?;

    // Migration helper for existing sqlite DBs missing retry_count column
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;")
        .execute(pool)
        .await;

    // How long before `due_at` to fire a reminder. Kept on the todo rather than
    // derived from the linked reminder so the web UI can read the current
    // setting back without reverse-engineering it from a timestamp difference.
    let _ = sqlx::query("ALTER TABLE todos ADD COLUMN remind_before_minutes INTEGER;")
        .execute(pool)
        .await;

    // Links a reminder back to the todo whose due date scheduled it, so the
    // reminder can be rescheduled or cancelled when that todo changes.
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN todo_id INTEGER;")
        .execute(pool)
        .await;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_reminders_todo ON reminders(todo_id);")
        .execute(pool)
        .await?;

    // `due_at` used to hold the rendered display string (`2026-12-31 23:59 UTC`),
    // which cannot be compared or subtracted from. Rewrite legacy rows into the
    // timestamp format sqlx reads back into `DateTime<Utc>`. The `strftime`
    // guard leaves anything unparseable alone rather than nulling the column.
    let _ = sqlx::query(
        r#"
        UPDATE todos
        SET due_at = strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', ''))
        WHERE due_at LIKE '% UTC'
          AND strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', '')) IS NOT NULL;
        "#,
    )
    .execute(pool)
    .await;

    // Clean up legacy matrix.org room_id placeholders
    let _ = sqlx::query("UPDATE reminders SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;
    let _ = sqlx::query("UPDATE todos SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;

    info!("Database migrations applied successfully.");
    Ok(())
}
