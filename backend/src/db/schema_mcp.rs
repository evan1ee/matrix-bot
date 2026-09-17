//! Schema for MCP personal access tokens.

use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
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
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_mcp_tokens_hash ON mcp_tokens(token_hash);")
        .execute(pool)
        .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_mcp_tokens_account ON mcp_tokens(account_id);")
        .execute(pool)
        .await?;

    Ok(())
}
