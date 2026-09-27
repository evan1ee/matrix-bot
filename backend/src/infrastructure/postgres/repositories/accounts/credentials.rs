//! Passkeys and WebAuthn credentials repository.

use chrono::{DateTime, Duration, Utc};

use crate::domain::accounts::{AccountId, DeleteOutcome, StoredCredential};

struct StoredCredentialRow {
    id: String,
    account_id: String,
    passkey_json: String,
    label: String,
    created_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
}

impl From<StoredCredentialRow> for StoredCredential {
    fn from(row: StoredCredentialRow) -> Self {
        StoredCredential {
            id: row.id,
            account_id: AccountId::new(row.account_id),
            passkey_json: row.passkey_json,
            label: row.label,
            created_at: row.created_at,
            last_used_at: row.last_used_at,
        }
    }
}

pub async fn add(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    credential_id: &str,
    passkey_json: &str,
    label: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
         VALUES ($1, $2, $3, $4)",
        credential_id,
        account_id.as_str(),
        passkey_json,
        label,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn list_for_account(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Vec<StoredCredential>, sqlx::Error> {
    let rows = sqlx::query_as!(
        StoredCredentialRow,
        "SELECT id, account_id, passkey_json, label, created_at, last_used_at \
         FROM webauthn_credentials WHERE account_id = $1 ORDER BY created_at",
        account_id.as_str(),
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn find(
    conn: impl sqlx::PgExecutor<'_>,
    credential_id: &str,
) -> Result<Option<StoredCredential>, sqlx::Error> {
    let row = sqlx::query_as!(
        StoredCredentialRow,
        "SELECT id, account_id, passkey_json, label, created_at, last_used_at \
         FROM webauthn_credentials WHERE id = $1",
        credential_id,
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn record_use(
    conn: impl sqlx::PgExecutor<'_>,
    credential_id: &str,
    passkey_json: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE webauthn_credentials SET passkey_json = $1, last_used_at = $2 WHERE id = $3",
        passkey_json,
        Utc::now(),
        credential_id,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn rename(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    credential_id: &str,
    label: &str,
) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query!(
        "UPDATE webauthn_credentials SET label = $1 WHERE id = $2 AND account_id = $3",
        label,
        credential_id,
        account_id.as_str(),
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

pub async fn count_for_account(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM webauthn_credentials WHERE account_id = $1"#,
        account_id.as_str(),
    )
    .fetch_one(conn)
    .await?;
    Ok(count)
}

pub async fn delete(
    pool: &sqlx::PgPool,
    account_id: &AccountId,
    credential_id: &str,
) -> Result<DeleteOutcome, sqlx::Error> {
    let mut tx = pool.begin().await?;

    // If only one passkey remains and no Matrix link exists, prevent self-lockout
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM webauthn_credentials WHERE account_id = $1"#,
        account_id.as_str(),
    )
    .fetch_one(&mut *tx)
    .await?;

    if count <= 1 {
        let has_matrix_link = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM matrix_links WHERE account_id = $1"#,
            account_id.as_str(),
        )
        .fetch_one(&mut *tx)
        .await?
            > 0;

        if !has_matrix_link {
            return Ok(DeleteOutcome::WouldLockOut);
        }
    }

    let affected = sqlx::query!(
        "DELETE FROM webauthn_credentials WHERE id = $1 AND account_id = $2",
        credential_id,
        account_id.as_str(),
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();

    tx.commit().await?;

    if affected > 0 {
        Ok(DeleteOutcome::Removed)
    } else {
        Ok(DeleteOutcome::NotFound)
    }
}

pub async fn issue_enrollment_token(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    ttl_minutes: i64,
) -> Result<String, sqlx::Error> {
    let token = format!("pk_{}", crate::utils::random::random_hex(20));
    let now = Utc::now();
    let expires_at = now + Duration::minutes(ttl_minutes);

    sqlx::query!(
        "INSERT INTO passkey_enrollment_tokens (token, account_id, created_at, expires_at) \
         VALUES ($1, $2, $3, $4)",
        token,
        account_id.as_str(),
        now,
        expires_at,
    )
    .execute(conn)
    .await?;

    Ok(token)
}

pub async fn validate_and_consume_enrollment_token(
    conn: impl sqlx::PgExecutor<'_>,
    token: &str,
) -> Result<Option<AccountId>, sqlx::Error> {
    let now = Utc::now();
    let account_id = sqlx::query_scalar!(
        "UPDATE passkey_enrollment_tokens \
         SET consumed_at = $1 \
         WHERE token = $2 AND consumed_at IS NULL AND expires_at > $1 \
           AND NOT EXISTS (SELECT 1 FROM webauthn_credentials c \
                           WHERE c.account_id = passkey_enrollment_tokens.account_id) \
         RETURNING account_id",
        now,
        token,
    )
    .fetch_optional(conn)
    .await?;

    Ok(account_id.map(AccountId::new))
}

#[cfg(test)]
pub async fn count_enrollment_tokens(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM passkey_enrollment_tokens WHERE account_id = $1"#,
        account_id.as_str(),
    )
    .fetch_one(conn)
    .await?;
    Ok(count)
}
