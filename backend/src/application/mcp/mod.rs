//! MCP service: minting, listing, resolving, and revoking MCP tokens.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::mcp::McpToken;
use crate::infrastructure::postgres::repositories::mcp as repo;

#[derive(Clone)]
pub struct McpService {
    pool: PgPool,
}

impl McpService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Mints a new MCP personal access token and returns the plaintext token and model.
    pub async fn create_token(
        &self,
        account_id: &AccountId,
        name: &str,
        expires_in_days: Option<i64>,
    ) -> Result<(String, McpToken), AppError> {
        if name.trim().is_empty() {
            return Err(AppError::Validation(
                "Token name cannot be empty".to_string(),
            ));
        }
        let (raw, token) =
            repo::create(&self.pool, account_id, name.trim(), expires_in_days).await?;
        Ok((raw, token))
    }

    /// Lists all tokens belonging to an account.
    pub async fn list_tokens(&self, account_id: &AccountId) -> Result<Vec<McpToken>, AppError> {
        let tokens = repo::list_for_account(&self.pool, account_id).await?;
        Ok(tokens)
    }

    /// Resolves a raw token to an unexpired token.
    pub async fn resolve_token(&self, raw_token: &str) -> Result<Option<McpToken>, AppError> {
        let token = repo::resolve(&self.pool, raw_token).await?;
        Ok(token)
    }

    /// Touches the last used timestamp of a token.
    pub async fn touch_last_used(&self, token_id: &str) -> Result<(), AppError> {
        repo::touch_last_used(&self.pool, token_id).await?;
        Ok(())
    }

    /// Revokes/deletes a token by ID.
    pub async fn delete_token(
        &self,
        account_id: &AccountId,
        token_id: &str,
    ) -> Result<bool, AppError> {
        let deleted = repo::delete(&self.pool, account_id, token_id).await?;
        Ok(deleted)
    }

    /// Revokes a token by ID or prefix.
    pub async fn revoke_by_id_or_prefix(
        &self,
        account_id: &AccountId,
        target: &str,
    ) -> Result<bool, AppError> {
        let deleted = repo::revoke_by_id_or_prefix(&self.pool, account_id, target).await?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate::run_migrations_unlocked, pool::seed_accounts, pool::test_pool};

    #[tokio::test]
    async fn mcp_service_validates_and_manages_tokens_with_owner_isolation() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        seed_accounts(&pool, &["mcp_svc_alice", "mcp_svc_bob"]).await;
        let alice = AccountId::from_seed("mcp_svc_alice");
        let bob = AccountId::from_seed("mcp_svc_bob");

        let svc = McpService::new(pool);

        // Empty or whitespace-only name is rejected
        let err = svc
            .create_token(&alice, "   ", None)
            .await
            .expect_err("blank token name must fail validation");
        assert!(matches!(err, AppError::Validation(_)));

        // Mint valid token with expiry
        let (raw, token) = svc
            .create_token(&alice, "  Cursor IDE  ", Some(30))
            .await
            .expect("create token");
        assert_eq!(token.name, "Cursor IDE");
        assert_eq!(token.account_id, alice);
        assert!(token.expires_at.is_some());

        // Resolve token
        let resolved = svc
            .resolve_token(&raw)
            .await
            .expect("resolve call")
            .expect("token should resolve");
        assert_eq!(resolved.id, token.id);

        // Touch last used
        svc.touch_last_used(&token.id)
            .await
            .expect("touch last used");
        let list = svc.list_tokens(&alice).await.expect("list alice tokens");
        assert_eq!(list.len(), 1);
        assert!(list[0].last_used_at.is_some());

        // Owner isolation: Bob cannot see or delete Alice's token
        assert!(svc.list_tokens(&bob).await.expect("list bob").is_empty());
        assert!(!svc
            .delete_token(&bob, &token.id)
            .await
            .expect("bob delete attempt"));
        assert!(!svc
            .revoke_by_id_or_prefix(&bob, &token.prefix[..10])
            .await
            .expect("bob revoke prefix attempt"));

        // Alice revokes by prefix
        assert!(svc
            .revoke_by_id_or_prefix(&alice, &raw[..10])
            .await
            .expect("alice revoke by prefix"));
        assert!(svc
            .resolve_token(&raw)
            .await
            .expect("resolve after revoke")
            .is_none());

        // Mint second token and delete by ID
        let (_, token2) = svc
            .create_token(&alice, "Claude Desktop", None)
            .await
            .expect("create second token");
        assert!(svc
            .delete_token(&alice, &token2.id)
            .await
            .expect("delete by id"));
        assert!(svc
            .list_tokens(&alice)
            .await
            .expect("list empty")
            .is_empty());
    }
}
