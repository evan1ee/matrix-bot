//! Passkeys and WebAuthn credentials repository.

use chrono::{DateTime, Duration, Utc};

use crate::domain::accounts::{AccountId, DeleteOutcome, StoredCredential};

struct StoredCredentialRow {
    id: String,
    account_id: uuid::Uuid,
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

/// The storage key of a credential: its raw id as URL-safe, unpadded base64.
///
/// This is the encoding of `webauthn_credentials.id`, so it lives with the
/// storage. Callers holding a raw id from an authenticator go through
/// `AccountService::find_passkey_by_raw_id` rather than encoding it themselves.
pub fn credential_id(raw: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
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
        account_id.as_uuid(),
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
        account_id.as_uuid(),
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
        account_id.as_uuid(),
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
        account_id.as_uuid(),
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
        account_id.as_uuid(),
    )
    .fetch_one(&mut *tx)
    .await?;

    if count <= 1 {
        let has_matrix_link = sqlx::query_scalar!(
            r#"SELECT COUNT(*) AS "count!" FROM matrix_links WHERE account_id = $1"#,
            account_id.as_uuid(),
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
        account_id.as_uuid(),
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
        account_id.as_uuid(),
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

    Ok(account_id.map(AccountId::from_uuid))
}

#[cfg(test)]
pub async fn count_enrollment_tokens(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM passkey_enrollment_tokens WHERE account_id = $1"#,
        account_id.as_uuid(),
    )
    .fetch_one(conn)
    .await?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPool;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_a", "acct_b", "acct_matrix", "acct_primary"])
            .await;
        pool
    }

    /// Inserts a credential row directly. Building a real `Passkey` needs an
    /// authenticator, so the shape of the blob is irrelevant to the ownership
    /// and lockout rules under test here.
    async fn add_raw(pool: &DbPool, id: &str, account_id: &str) {
        let acct = AccountId::from_seed(account_id);
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'test', 'active') ON CONFLICT DO NOTHING",
            acct.as_uuid()
        )
        .execute(pool)
        .await
        .unwrap();

        sqlx::query!(
            "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
             VALUES ($1, $2, '{}', 'test key')",
            id,
            acct.as_uuid()
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[test]
    fn a_credential_id_is_url_safe_unpadded_base64() {
        assert_eq!(credential_id(&[0xfb, 0xff, 0xfe, 0x01]), "-__-AQ");
        assert_eq!(credential_id(&[]), "");
    }

    #[tokio::test]
    async fn test_credentials_are_listed_per_account() {
        let pool = db().await;
        add_raw(&pool, "cred_a1", "acct_a").await;
        add_raw(&pool, "cred_a2", "acct_a").await;
        add_raw(&pool, "cred_b1", "acct_b").await;

        let mine = list_for_account(&pool, &AccountId::from_seed("acct_a"))
            .await
            .unwrap();
        assert_eq!(mine.len(), 2);
        assert!(mine
            .iter()
            .all(|c| c.account_id == AccountId::from_seed("acct_a")));
    }

    /// Removing the only passkey would lock the account out of itself.
    #[tokio::test]
    async fn test_the_last_passkey_cannot_be_deleted() {
        let pool = db().await;
        let acct = AccountId::from_seed("acct_a");
        add_raw(&pool, "cred_only", "acct_a").await;

        let outcome = delete(&pool, &acct, "cred_only").await.unwrap();
        assert_eq!(outcome, DeleteOutcome::WouldLockOut);
        assert_eq!(count_for_account(&pool, &acct).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_a_passkey_can_be_deleted_once_a_second_exists() {
        let pool = db().await;
        let acct = AccountId::from_seed("acct_a");
        add_raw(&pool, "cred_1", "acct_a").await;
        add_raw(&pool, "cred_2", "acct_a").await;

        let outcome = delete(&pool, &acct, "cred_1").await.unwrap();
        assert_eq!(outcome, DeleteOutcome::Removed);
        assert_eq!(count_for_account(&pool, &acct).await.unwrap(), 1);
    }

    /// A credential id is not a capability: knowing one must not let another
    /// account delete or rename it.
    #[tokio::test]
    async fn test_one_account_cannot_touch_anothers_passkey() {
        let pool = db().await;
        add_raw(&pool, "cred_a1", "acct_a").await;
        add_raw(&pool, "cred_a2", "acct_a").await;
        add_raw(&pool, "cred_b1", "acct_b").await;
        add_raw(&pool, "cred_b2", "acct_b").await;
        let (a, b) = (
            AccountId::from_seed("acct_a"),
            AccountId::from_seed("acct_b"),
        );

        let outcome = delete(&pool, &b, "cred_a1").await.unwrap();
        assert_eq!(outcome, DeleteOutcome::NotFound);
        assert_eq!(count_for_account(&pool, &a).await.unwrap(), 2);

        let renamed = rename(&pool, &b, "cred_a1", "stolen").await.unwrap();
        assert!(
            !renamed,
            "renaming another account's passkey must not succeed"
        );
    }

    /// A credential blob that will not parse stays listed, so the UI can show
    /// it and offer removal; the service skips it when authenticating.
    #[tokio::test]
    async fn test_an_unreadable_credential_is_still_listed() {
        let pool = db().await;
        add_raw(&pool, "cred_broken", "acct_a").await;

        let listed = list_for_account(&pool, &AccountId::from_seed("acct_a"))
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].passkey().is_none());
    }

    #[tokio::test]
    async fn test_enrollment_token_issue_and_consumption() {
        let pool = db().await;
        let acct = AccountId::from_seed("acct_matrix");
        let token = issue_enrollment_token(&pool, &acct, 10).await.unwrap();
        assert!(!token.is_empty());

        // First consumption succeeds
        let consumed = validate_and_consume_enrollment_token(&pool, &token)
            .await
            .unwrap();
        assert_eq!(consumed, Some(acct));

        // Second consumption fails (single-use)
        let second = validate_and_consume_enrollment_token(&pool, &token)
            .await
            .unwrap();
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn test_last_passkey_can_be_deleted_if_matrix_linked() {
        let pool = db().await;
        add_raw(&pool, "cred_only", "acct_primary").await;

        // Insert matrix link
        let acct = AccountId::from_seed("acct_primary");
        sqlx::query!(
            "INSERT INTO matrix_links (mxid, account_id) VALUES ('@alice:matrix.org', $1)",
            acct.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        // Deleting the only passkey succeeds because Matrix is primary identity
        let outcome = delete(&pool, &acct, "cred_only").await.unwrap();
        assert_eq!(outcome, DeleteOutcome::Removed);
        assert_eq!(count_for_account(&pool, &acct).await.unwrap(), 0);
    }
}
