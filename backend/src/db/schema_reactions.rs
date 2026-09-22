use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS bot_message_targets (
            event_id TEXT PRIMARY KEY,
            room_id TEXT NOT NULL,
            entity_type TEXT NOT NULL,
            entity_id BIGINT NOT NULL,
            owner_id TEXT NOT NULL,
            title TEXT NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    crate::db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_bot_msg_targets_entity ON bot_message_targets(entity_type, entity_id);")?;

    Ok(())
}
