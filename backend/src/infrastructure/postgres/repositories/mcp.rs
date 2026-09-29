use chrono::{Duration, Utc};
use uuid::Uuid;

use crate::accounts::session::hash_token;
use crate::domain::accounts::AccountId;
use crate::domain::mcp::McpToken;

/// Mints a new MCP personal access token and stores its hash in the database.
/// Returns the plaintext token (only returned at creation time) and the model row.
pub async fn create(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    name: &str,
    expires_in_days: Option<i64>,
) -> Result<(String, McpToken), sqlx::Error> {
    let random_hex: String = crate::utils::random::random_hex(20);

    let token = format!("mb_mcp_{}", random_hex);
    let token_hash = hash_token(&token);
    let id = format!("tok_{}", Uuid::new_v4().simple());
    let prefix = format!("{}...", &token[..14]);
    let now = crate::utils::time::now_at_storage_precision();
    let expires_at = expires_in_days.map(|days| now + Duration::days(days));

    sqlx::query!(
        r#"
        INSERT INTO mcp_tokens (id, account_id, name, token_hash, prefix, created_at, expires_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        "#,
        id,
        account_id.as_uuid(),
        name,
        token_hash,
        prefix,
        now,
        expires_at
    )
    .execute(conn)
    .await?;

    let row = McpToken {
        id,
        account_id: *account_id,
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
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Vec<McpToken>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, account_id, name, token_hash, prefix, created_at, last_used_at, expires_at
        FROM mcp_tokens
        WHERE account_id = $1
        ORDER BY created_at DESC
        "#,
        account_id.as_uuid()
    )
    .fetch_all(conn)
    .await?;

    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(|r| McpToken {
            id: r.id,
            account_id: AccountId::from_uuid(r.account_id),
            name: r.name,
            token_hash: r.token_hash,
            prefix: r.prefix,
            created_at: r.created_at.unwrap_or(now),
            last_used_at: r.last_used_at,
            expires_at: r.expires_at,
        })
        .collect())
}

/// Resolves a plaintext Bearer token to a live, unexpired token model.
pub async fn resolve(
    conn: impl sqlx::PgExecutor<'_>,
    plaintext_token: &str,
) -> Result<Option<McpToken>, sqlx::Error> {
    let token_hash = hash_token(plaintext_token);
    let row = sqlx::query!(
        r#"
        SELECT id, account_id, name, token_hash, prefix, created_at, last_used_at, expires_at
        FROM mcp_tokens
        WHERE token_hash = $1
        "#,
        token_hash
    )
    .fetch_optional(conn)
    .await?;

    let Some(r) = row else {
        return Ok(None);
    };
    let token = McpToken {
        id: r.id,
        account_id: AccountId::from_uuid(r.account_id),
        name: r.name,
        token_hash: r.token_hash,
        prefix: r.prefix,
        created_at: r.created_at.unwrap_or_else(Utc::now),
        last_used_at: r.last_used_at,
        expires_at: r.expires_at,
    };

    if token.is_expired(Utc::now()) {
        Ok(None)
    } else {
        Ok(Some(token))
    }
}

/// Asynchronously updates `last_used_at` timestamp.
pub async fn touch_last_used(conn: impl sqlx::PgExecutor<'_>, id: &str) -> Result<(), sqlx::Error> {
    let now = crate::utils::time::now_at_storage_precision();
    sqlx::query!(
        r#"UPDATE mcp_tokens SET last_used_at = $1 WHERE id = $2"#,
        now,
        id
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes a token by ID, scoped to owner.
pub async fn delete(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    token_id: &str,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"DELETE FROM mcp_tokens WHERE account_id = $1 AND id = $2"#,
        account_id.as_uuid(),
        token_id
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Revokes a token by exact ID or prefix match.
pub async fn revoke_by_id_or_prefix(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    target: &str,
) -> Result<bool, sqlx::Error> {
    let prefix_query = format!("{}%", target.trim_end_matches('.'));
    let res = sqlx::query!(
        r#"DELETE FROM mcp_tokens WHERE account_id = $1 AND (id = $2 OR prefix LIKE $3 OR token_hash = $2)"#,
        account_id.as_uuid(),
        target,
        prefix_query
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub struct McpTokenRepository;

impl McpTokenRepository {
    pub async fn create(
        conn: impl sqlx::PgExecutor<'_>,
        account_id: &AccountId,
        name: &str,
        expires_in_days: Option<i64>,
    ) -> Result<(String, McpToken), sqlx::Error> {
        create(conn, account_id, name, expires_in_days).await
    }

    pub async fn list_for_account(
        conn: impl sqlx::PgExecutor<'_>,
        account_id: &AccountId,
    ) -> Result<Vec<McpToken>, sqlx::Error> {
        list_for_account(conn, account_id).await
    }

    pub async fn resolve(
        conn: impl sqlx::PgExecutor<'_>,
        raw_token: &str,
    ) -> Result<Option<McpToken>, sqlx::Error> {
        resolve(conn, raw_token).await
    }

    pub async fn touch_last_used(
        conn: impl sqlx::PgExecutor<'_>,
        id: &str,
    ) -> Result<(), sqlx::Error> {
        touch_last_used(conn, id).await
    }

    pub async fn delete(
        conn: impl sqlx::PgExecutor<'_>,
        account_id: &AccountId,
        token_id: &str,
    ) -> Result<bool, sqlx::Error> {
        delete(conn, account_id, token_id).await
    }

    pub async fn revoke_by_id_or_prefix(
        conn: impl sqlx::PgExecutor<'_>,
        account_id: &AccountId,
        target: &str,
    ) -> Result<bool, sqlx::Error> {
        revoke_by_id_or_prefix(conn, account_id, target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mcp_token_repository_lifecycle() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let acct_id = AccountId::from_seed("mcp_test");
        sqlx::query!(
            "INSERT INTO accounts (id, display_name) VALUES ($1, 'Alice')",
            acct_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        // Mint token
        let (raw_token, token) = create(&pool, &acct_id, "Claude Test", None).await.unwrap();

        assert!(raw_token.starts_with("mb_mcp_"));
        assert_eq!(token.account_id, acct_id);
        assert_eq!(token.name, "Claude Test");

        // Resolve token
        let resolved = resolve(&pool, &raw_token)
            .await
            .unwrap()
            .expect("must resolve valid token");
        assert_eq!(resolved.id, token.id);

        // List
        let list = list_for_account(&pool, &acct_id).await.unwrap();
        assert_eq!(list.len(), 1);

        // Touch last used
        touch_last_used(&pool, &token.id).await.unwrap();
        let list_after_touch = list_for_account(&pool, &acct_id).await.unwrap();
        assert!(list_after_touch[0].last_used_at.is_some());

        // Revoke
        let revoked = delete(&pool, &acct_id, &token.id).await.unwrap();
        assert!(revoked);

        // Resolve after revoke returns None
        let resolved_after = resolve(&pool, &raw_token).await.unwrap();
        assert!(resolved_after.is_none());
    }
}
