use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Per-user display preferences, keyed by owner_id.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_settings (
            owner_id TEXT PRIMARY KEY,
            timezone TEXT NOT NULL DEFAULT 'UTC',
            currency TEXT NOT NULL DEFAULT 'USD',
            dev_room_id TEXT,
            max_history_turns INTEGER NOT NULL DEFAULT 10,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::query("ALTER TABLE user_settings ADD COLUMN dev_room_id TEXT;")
        .execute(pool)
        .await;

    let _ = sqlx::query(
        "ALTER TABLE user_settings ADD COLUMN max_history_turns INTEGER NOT NULL DEFAULT 10;",
    )
    .execute(pool)
    .await;

    Ok(())
}
