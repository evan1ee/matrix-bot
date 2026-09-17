use chrono::{Duration, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::accounts::session::hash_token;
use crate::modules::mcp::models::McpToken;

pub struct McpTokenRepository;

impl McpTokenRepository {
    /// Mints a new MCP personal access token and stores its hash in SQLite.
    /// Returns the plaintext token (only returned at creation time) and the model row.
    pub async fn create(
        pool: &SqlitePool,
        account_id: &str,
        name: &str,
        expires_in_days: Option<i64>,
    ) -> Result<(String, McpToken), sqlx::Error> {
        let random_hex: String =
            sqlx::query_scalar("SELECT lower(hex(randomblob(20)))")
                .fetch_one(pool)
                .await?;

        let token = format!("mb_mcp_{}", random_hex);
        let token_hash = hash_token(&token);
        let id = format!("tok_{}", Uuid::new_v4().simple());
        let prefix = format!("{}...", &token[..14]);
        let now = Utc::now();
        let expires_at = expires_in_days.map(|days| now + Duration::days(days));

        sqlx::query(
            "INSERT INTO mcp_tokens (id, account_id, name, token_hash, prefix, created_at, expires_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(account_id)
        .bind(name)
        .bind(&token_hash)
        .bind(&prefix)
        .bind(now)
        .bind(expires_at)
        .execute(pool)
        .await?;

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
        pool: &SqlitePool,
        account_id: &str,
    ) -> Result<Vec<McpToken>, sqlx::Error> {
        sqlx::query_as::<_, McpToken>(
            "SELECT * FROM mcp_tokens WHERE account_id = ? ORDER BY created_at DESC",
        )
        .bind(account_id)
        .fetch_all(pool)
        .await
    }

    /// Resolves a plaintext Bearer token to a live, unexpired token model.
    pub async fn resolve(
        pool: &SqlitePool,
        plaintext_token: &str,
    ) -> Result<Option<McpToken>, sqlx::Error> {
        let token_hash = hash_token(plaintext_token);
        let token: Option<McpToken> = sqlx::query_as::<_, McpToken>(
            "SELECT * FROM mcp_tokens WHERE token_hash = ?",
        )
        .bind(&token_hash)
        .fetch_optional(pool)
        .await?;

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
    pub async fn touch_last_used(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
        let now = Utc::now();
        sqlx::query("UPDATE mcp_tokens SET last_used_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Deletes a token by ID, scoped to owner.
    pub async fn delete(
        pool: &SqlitePool,
        account_id: &str,
        token_id: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM mcp_tokens WHERE account_id = ? AND id = ?")
            .bind(account_id)
            .bind(token_id)
            .execute(pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Revokes a token by exact ID or prefix match.
    pub async fn revoke_by_id_or_prefix(
        pool: &SqlitePool,
        account_id: &str,
        target: &str,
    ) -> Result<bool, sqlx::Error> {
        let prefix_query = format!("{}%", target.trim_end_matches('.'));
        let result = sqlx::query(
            "DELETE FROM mcp_tokens WHERE account_id = ? AND (id = ? OR prefix LIKE ? OR token_hash = ?)",
        )
        .bind(account_id)
        .bind(target)
        .bind(&prefix_query)
        .bind(target)
        .execute(pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_token_lifecycle() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema_auth::create_tables(&pool).await.unwrap();
        crate::db::schema_mcp::create_tables(&pool).await.unwrap();

        // Create a test account
        sqlx::query("INSERT INTO accounts (id, display_name) VALUES ('acct_1', 'Alice')")
            .execute(&pool)
            .await
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
