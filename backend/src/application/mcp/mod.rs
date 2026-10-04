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
