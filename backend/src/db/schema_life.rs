use sqlx::SqlitePool;

pub async fn run_life_schema(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS life_profiles (
            owner_id TEXT PRIMARY KEY,
            birth_date TEXT NOT NULL,
            expected_lifespan_years INTEGER NOT NULL DEFAULT 80,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS life_day_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            log_date TEXT NOT NULL,
            mood TEXT NOT NULL,
            weather TEXT,
            activities TEXT,
            note TEXT,
            time_allocation TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            UNIQUE(owner_id, log_date)
        );

        CREATE INDEX IF NOT EXISTS idx_life_day_logs_owner_date ON life_day_logs(owner_id, log_date);
        CREATE INDEX IF NOT EXISTS idx_life_day_logs_owner_mood ON life_day_logs(owner_id, mood);
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::query("ALTER TABLE life_day_logs ADD COLUMN time_allocation TEXT;")
        .execute(pool)
        .await;

    Ok(())
}
