use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS bot_message_targets (
            event_id TEXT PRIMARY KEY,
            room_id TEXT NOT NULL,
            entity_type TEXT NOT NULL,
            entity_id INTEGER NOT NULL,
            owner_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_bot_msg_targets_entity ON bot_message_targets(entity_type, entity_id);",
    )
    .execute(pool)
    .await?;

    Ok(())
}
