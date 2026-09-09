use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS habits (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            frequency TEXT NOT NULL DEFAULT 'daily',
            category TEXT NOT NULL DEFAULT 'wellness',
            target_days_per_week INTEGER NOT NULL DEFAULT 7,
            archived BOOLEAN NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        CREATE INDEX IF NOT EXISTS idx_habits_owner ON habits(owner_id);
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS habit_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            habit_id INTEGER NOT NULL REFERENCES habits(id) ON DELETE CASCADE,
            owner_id TEXT NOT NULL,
            log_date TEXT NOT NULL,
            completed BOOLEAN NOT NULL DEFAULT 1,
            note TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(habit_id, log_date)
        );
        CREATE INDEX IF NOT EXISTS idx_habit_logs_owner_date ON habit_logs(owner_id, log_date);
        CREATE INDEX IF NOT EXISTS idx_habit_logs_habit_date ON habit_logs(habit_id, log_date);
        "#,
    )
    .execute(pool)
    .await?;

    Ok(())
}
