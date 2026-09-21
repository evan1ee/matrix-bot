//! Schema for MCP personal access tokens.

use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS mcp_tokens (
            id            TEXT PRIMARY KEY,
            account_id    TEXT NOT NULL,
            name          TEXT NOT NULL,
            token_hash    TEXT NOT NULL UNIQUE,
            prefix        TEXT NOT NULL,
            created_at    TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
            last_used_at  TIMESTAMPTZ,
            expires_at    TIMESTAMPTZ,
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_mcp_tokens_hash ON mcp_tokens(token_hash);"
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_mcp_tokens_account ON mcp_tokens(account_id);"
    )?;

    Ok(())
}
