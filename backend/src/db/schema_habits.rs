use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS habits (
            id {{PK}},
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT,
            frequency TEXT NOT NULL DEFAULT 'daily',
            category TEXT NOT NULL DEFAULT 'wellness',
            target_days_per_week BIGINT NOT NULL DEFAULT 7,
            archived BOOLEAN NOT NULL DEFAULT FALSE,
            created_at TEXT DEFAULT {{NOW_TEXT}},
            updated_at TEXT DEFAULT {{NOW_TEXT}}
        );
        CREATE INDEX IF NOT EXISTS idx_habits_owner ON habits(owner_id);
        "#
    )?;

    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS habit_logs (
            id {{PK}},
            habit_id BIGINT NOT NULL REFERENCES habits(id) ON DELETE CASCADE,
            owner_id TEXT NOT NULL,
            log_date TEXT NOT NULL,
            completed BOOLEAN NOT NULL DEFAULT TRUE,
            note TEXT,
            created_at TEXT DEFAULT {{NOW_TEXT}},
            UNIQUE(habit_id, log_date)
        );
        CREATE INDEX IF NOT EXISTS idx_habit_logs_owner_date ON habit_logs(owner_id, log_date);
        CREATE INDEX IF NOT EXISTS idx_habit_logs_habit_date ON habit_logs(habit_id, log_date);
        "#
    )?;

    Ok(())
}
