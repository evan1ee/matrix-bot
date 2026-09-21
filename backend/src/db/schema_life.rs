use crate::db::DbPool;
pub async fn run_life_schema(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS life_profiles (
            owner_id TEXT PRIMARY KEY,
            birth_date TEXT NOT NULL,
            expected_lifespan_years BIGINT NOT NULL DEFAULT 80,
            created_at TEXT NOT NULL DEFAULT {{NOW_TEXT}},
            updated_at TEXT NOT NULL DEFAULT {{NOW_TEXT}}
        );

        CREATE TABLE IF NOT EXISTS life_day_logs (
            id {{PK}},
            owner_id TEXT NOT NULL,
            log_date TEXT NOT NULL,
            mood TEXT NOT NULL,
            weather TEXT,
            activities TEXT,
            note TEXT,
            time_allocation TEXT,
            created_at TEXT NOT NULL DEFAULT {{NOW_TEXT}},
            updated_at TEXT NOT NULL DEFAULT {{NOW_TEXT}},
            UNIQUE(owner_id, log_date)
        );

        CREATE INDEX IF NOT EXISTS idx_life_day_logs_owner_date ON life_day_logs(owner_id, log_date);
        CREATE INDEX IF NOT EXISTS idx_life_day_logs_owner_mood ON life_day_logs(owner_id, mood);
        "#
    )?;

    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE life_day_logs ADD COLUMN time_allocation TEXT;"
    );

    Ok(())
}
