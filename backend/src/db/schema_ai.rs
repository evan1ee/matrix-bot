use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS ai_user_configs (
            owner_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
            api_base_url TEXT,
            api_key TEXT,
            model_name TEXT,
            system_prompt TEXT,
            temperature REAL DEFAULT 0.7,
            max_tokens BIGINT DEFAULT 2048,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE TABLE IF NOT EXISTS ai_conversations (
            id TEXT PRIMARY KEY,
            owner_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            room_id TEXT NOT NULL,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_ai_conversations_owner_room 
        ON ai_conversations(owner_id, room_id, updated_at);

        CREATE TABLE IF NOT EXISTS ai_messages (
            id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL REFERENCES ai_conversations(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool')),
            content TEXT NOT NULL,
            tool_call_id TEXT,
            tool_name TEXT,
            created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );

        CREATE INDEX IF NOT EXISTS idx_ai_messages_conv_created 
        ON ai_messages(conversation_id, created_at);
        "#
    )?;

    Ok(())
}
