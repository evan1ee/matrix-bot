//! Schema for MCP personal access tokens.

use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS mcp_tokens (
            id            TEXT PRIMARY KEY,
            account_id    TEXT NOT NULL,
            name          TEXT NOT NULL,
            token_hash    TEXT NOT NULL UNIQUE,
            prefix        TEXT NOT NULL,
            created_at    TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            last_used_at  TIMESTAMP,
            expires_at    TIMESTAMP,
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_mcp_tokens_hash ON mcp_tokens(token_hash);"
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_mcp_tokens_account ON mcp_tokens(account_id);"
    )?;

    Ok(())
}
