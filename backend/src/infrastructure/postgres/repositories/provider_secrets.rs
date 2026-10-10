//! Persistence for `provider_secrets` (encrypted credential storage).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretMetadataRow {
    pub id: uuid::Uuid,
    pub provider: String,
    pub name: String,
    pub status: String,
    pub key_version: i32,
    pub last_tested_at: Option<DateTime<Utc>>,
    pub last_test_status: Option<String>,
    pub last_test_error: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSecretRow {
    pub id: uuid::Uuid,
    pub provider: String,
    pub name: String,
    pub secret_ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub key_version: i32,
    pub status: String,
    pub updated_by: Option<uuid::Uuid>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

/// Lists non-sensitive metadata for all configured provider secrets.
pub async fn list_secret_metadata(
    conn: impl PgExecutor<'_>,
) -> Result<Vec<SecretMetadataRow>, sqlx::Error> {
    sqlx::query_as!(
        SecretMetadataRow,
        "SELECT id, provider, name, status, key_version, last_tested_at, \
         last_test_status, last_test_error, created_at, updated_at, metadata \
         FROM provider_secrets ORDER BY provider, name",
    )
    .fetch_all(conn)
    .await
}

/// Retrieves an encrypted secret by its ID.
pub async fn get_secret_by_id(
    conn: impl PgExecutor<'_>,
    id: uuid::Uuid,
) -> Result<Option<EncryptedSecretRow>, sqlx::Error> {
    sqlx::query_as!(
        EncryptedSecretRow,
        "SELECT id, provider, name, secret_ciphertext, nonce, key_version, \
         status, updated_by, created_at, updated_at, metadata \
         FROM provider_secrets WHERE id = $1",
        id,
    )
    .fetch_optional(conn)
    .await
}

/// Retrieves an encrypted secret by provider and slot name.
pub async fn get_secret_by_provider(
    conn: impl PgExecutor<'_>,
    provider: &str,
    name: &str,
) -> Result<Option<EncryptedSecretRow>, sqlx::Error> {
    sqlx::query_as!(
        EncryptedSecretRow,
        "SELECT id, provider, name, secret_ciphertext, nonce, key_version, \
         status, updated_by, created_at, updated_at, metadata \
         FROM provider_secrets WHERE provider = $1 AND name = $2",
        provider,
        name,
    )
    .fetch_optional(conn)
    .await
}

/// Inserts a new encrypted secret row, returning its ID.
pub async fn insert_secret(
    conn: impl PgExecutor<'_>,
    provider: &str,
    name: &str,
    ciphertext: &[u8],
    nonce: &[u8],
    key_version: i32,
    updated_by: Option<&AccountId>,
    metadata: Option<&serde_json::Value>,
) -> Result<uuid::Uuid, sqlx::Error> {
    let updated_by_uuid = updated_by.map(|a| a.uuid());
    let row: uuid::Uuid = sqlx::query_scalar!(
        "INSERT INTO provider_secrets (provider, name, secret_ciphertext, nonce, \
         key_version, updated_by, metadata) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
        provider,
        name,
        ciphertext,
        nonce,
        key_version,
        updated_by_uuid,
        metadata,
    )
    .fetch_one(conn)
    .await?;
    Ok(row)
}

/// Updates ciphertext, nonce, key version, status, and metadata for an existing secret.
pub async fn update_secret(
    conn: impl PgExecutor<'_>,
    id: uuid::Uuid,
    ciphertext: &[u8],
    nonce: &[u8],
    key_version: i32,
    status: Option<&str>,
    updated_by: Option<&AccountId>,
    metadata: Option<&serde_json::Value>,
) -> Result<bool, sqlx::Error> {
    let updated_by_uuid = updated_by.map(|a| a.uuid());
    let res = sqlx::query!(
        "UPDATE provider_secrets SET \
            secret_ciphertext = $2, \
            nonce = $3, \
            key_version = $4, \
            status = COALESCE($5, status), \
            updated_by = $6, \
            metadata = COALESCE($7, metadata), \
            updated_at = CURRENT_TIMESTAMP \
         WHERE id = $1",
        id,
        ciphertext,
        nonce,
        key_version,
        status,
        updated_by_uuid,
        metadata,
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Updates connectivity test status outcome for a credential.
pub async fn update_test_status(
    conn: impl PgExecutor<'_>,
    id: uuid::Uuid,
    status: &str,
    error: Option<&str>,
    tested_at: DateTime<Utc>,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "UPDATE provider_secrets SET \
            last_tested_at = $2, \
            last_test_status = $3, \
            last_test_error = $4 \
         WHERE id = $1",
        id,
        tested_at,
        status,
        error,
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Deletes an encrypted credential by ID.
pub async fn delete_secret(conn: impl PgExecutor<'_>, id: uuid::Uuid) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!("DELETE FROM provider_secrets WHERE id = $1", id)
        .execute(conn)
        .await?;
    Ok(res.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn pool() -> PgPool {
        let p = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&p).await;
        p
    }

    #[tokio::test]
    async fn test_provider_secrets_crud_and_unique_constraint() {
        let p = pool().await;
        let admin = AccountId::from_seed("admin");
        crate::db::pool::seed_account(&p, &admin).await;

        let ciphertext = vec![1, 2, 3, 4];
        let nonce = vec![5, 6, 7, 8];

        let id = insert_secret(
            &p,
            "openai",
            "default",
            &ciphertext,
            &nonce,
            1,
            Some(&admin),
            None,
        )
        .await
        .unwrap();

        let found = get_secret_by_id(&p, id).await.unwrap().unwrap();
        assert_eq!(found.provider, "openai");
        assert_eq!(found.name, "default");
        assert_eq!(found.secret_ciphertext, ciphertext);
        assert_eq!(found.nonce, nonce);

        // Conflict on duplicate (provider, name)
        let dup = insert_secret(
            &p,
            "openai",
            "default",
            &ciphertext,
            &nonce,
            1,
            Some(&admin),
            None,
        )
        .await;
        assert!(dup.is_err());

        // Update test status
        let now = Utc::now();
        let ok = update_test_status(&p, id, "healthy", None, now)
            .await
            .unwrap();
        assert!(ok);

        let list = list_secret_metadata(&p).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].last_test_status.as_deref(), Some("healthy"));

        // Delete
        let deleted = delete_secret(&p, id).await.unwrap();
        assert!(deleted);
        assert!(get_secret_by_id(&p, id).await.unwrap().is_none());
    }
}
