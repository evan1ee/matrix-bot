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
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    info!("Database migrations applied successfully.");
    Ok(())
}
