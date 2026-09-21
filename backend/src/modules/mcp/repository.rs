use crate::db::DbPool;
use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::accounts::session::hash_token;
use crate::modules::mcp::models::McpToken;

pub struct McpTokenRepository;

impl McpTokenRepository {
    /// Mints a new MCP personal access token and stores its hash in SQLite.
    /// Returns the plaintext token (only returned at creation time) and the model row.
    pub async fn create(
        pool: &DbPool,
        account_id: &str,
        name: &str,
        expires_in_days: Option<i64>,
    ) -> Result<(String, McpToken), sqlx::Error> {
        let random_hex: String =
            crate::db_scalar_one!(pool, _, "SELECT lower(hex(randomblob(20)))")?;

        let token = format!("mb_mcp_{}", random_hex);
        let token_hash = hash_token(&token);
        let id = format!("tok_{}", Uuid::new_v4().simple());
        let prefix = format!("{}...", &token[..14]);
        let now = Utc::now();
        let expires_at = expires_in_days.map(|days| now + Duration::days(days));

        crate::db_execute!(pool, "INSERT INTO mcp_tokens (id, account_id, name, token_hash, prefix, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)", &id, account_id, name, &token_hash, &prefix, now, expires_at)?;

        let row = McpToken {
            id,
            account_id: account_id.to_string(),
            name: name.to_string(),
            token_hash,
            prefix,
            created_at: now,
            last_used_at: None,
            expires_at,
        };

        Ok((token, row))
    }

    /// Lists all tokens belonging to an account.
    pub async fn list_for_account(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Vec<McpToken>, sqlx::Error> {
        crate::db_fetch_all!(
            pool,
            McpToken,
            "SELECT * FROM mcp_tokens WHERE account_id = ? ORDER BY created_at DESC",
            account_id
        )
    }

    /// Resolves a plaintext Bearer token to a live, unexpired token model.
    pub async fn resolve(
        pool: &DbPool,
        plaintext_token: &str,
    ) -> Result<Option<McpToken>, sqlx::Error> {
        let token_hash = hash_token(plaintext_token);
        let token: Option<McpToken> = crate::db_fetch_optional!(
            pool,
            McpToken,
            "SELECT * FROM mcp_tokens WHERE token_hash = ?",
            &token_hash
        )?;

        if let Some(tok) = token {
            if tok.is_expired(Utc::now()) {
                return Ok(None);
            }
            Ok(Some(tok))
        } else {
            Ok(None)
        }
    }

    /// Asynchronously updates `last_used_at` timestamp.
    pub async fn touch_last_used(pool: &DbPool, id: &str) -> Result<(), sqlx::Error> {
        let now = Utc::now();
        crate::db_execute!(
            pool,
            "UPDATE mcp_tokens SET last_used_at = ? WHERE id = ?",
            now,
            id
        )?;
        Ok(())
    }

    /// Deletes a token by ID, scoped to owner.
    pub async fn delete(
        pool: &DbPool,
        account_id: &str,
        token_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = crate::db_execute!(
            pool,
            "DELETE FROM mcp_tokens WHERE account_id = ? AND id = ?",
            account_id,
            token_id
        )?;
        Ok(result > 0)
    }

    /// Revokes a token by exact ID or prefix match.
    pub async fn revoke_by_id_or_prefix(
        pool: &DbPool,
        account_id: &str,
        target: &str,
    ) -> Result<bool, sqlx::Error> {
        let prefix_query = format!("{}%", target.trim_end_matches('.'));
        let result = crate::db_execute!(pool, "DELETE FROM mcp_tokens WHERE account_id = ? AND (id = ? OR prefix LIKE ? OR token_hash = ?)", account_id, target, &prefix_query, target)?;
        Ok(result > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_mcp_token_lifecycle() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db::schema_auth::create_tables(&pool).await.unwrap();
        crate::db::schema_mcp::create_tables(&pool).await.unwrap();

        // Create a test account
        crate::db_execute!(
            &pool,
            "INSERT INTO accounts (id, display_name) VALUES ('acct_1', 'Alice')"
        )
        .unwrap();

        // Mint token
        let (raw_token, token) = McpTokenRepository::create(&pool, "acct_1", "Claude Test", None)
            .await
            .unwrap();

        assert!(raw_token.starts_with("mb_mcp_"));
        assert_eq!(token.account_id, "acct_1");
        assert_eq!(token.name, "Claude Test");

        // Resolve token
        let resolved = McpTokenRepository::resolve(&pool, &raw_token)
            .await
            .unwrap()
            .expect("must resolve valid token");
        assert_eq!(resolved.id, token.id);

        // List
        let list = McpTokenRepository::list_for_account(&pool, "acct_1")
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        // Revoke
        let revoked = McpTokenRepository::delete(&pool, "acct_1", &token.id)
            .await
            .unwrap();
        assert!(revoked);

        // Resolve after revoke returns None
        let resolved_after = McpTokenRepository::resolve(&pool, &raw_token)
            .await
            .unwrap();
        assert!(resolved_after.is_none());
    }
}
