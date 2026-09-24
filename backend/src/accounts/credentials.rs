//! Stored passkeys.
//!
//! A row here holds a `webauthn_rs` [`Passkey`] serialised whole. It is kept as
//! one JSON blob rather than picked apart into columns because the crate owns
//! that shape — it carries the public key, the algorithm, the signature
//! counter and the backup-eligibility flags, and every one of those is the
//! library's business to interpret, not this code's to mirror.
//!
//! Nothing secret lives here. A passkey's private half never leaves the
//! authenticator, which is exactly why this replaces `password_hash`: there is
//! no longer anything in the database worth stealing to sign in as someone.

use crate::db::DbPool;
use chrono::{DateTime, Utc};
use webauthn_rs::prelude::Passkey;

/// A registered passkey, as the account settings screen shows it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct StoredCredential {
    pub id: String,
    pub account_id: String,
    pub passkey_json: String,
    pub label: String,
    pub created_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl StoredCredential {
    /// The `webauthn_rs` credential this row carries.
    ///
    /// A row that will not deserialise is skipped rather than fatal: one
    /// unreadable credential must not lock a user out of an account whose other
    /// passkeys are fine.
    pub fn passkey(&self) -> Option<Passkey> {
        match serde_json::from_str(&self.passkey_json) {
            Ok(passkey) => Some(passkey),
            Err(err) => {
                tracing::error!("Credential {} will not deserialise: {:?}", self.id, err);
                None
            }
        }
    }
}

/// The stable text form of a credential ID.
pub fn credential_id_string(id: impl AsRef<[u8]>) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.as_ref())
}

pub struct CredentialRepository;

impl CredentialRepository {
    pub async fn add(
        pool: &DbPool,
        account_id: &str,
        passkey: &Passkey,
        label: &str,
    ) -> Result<String, sqlx::Error> {
        let id = credential_id_string(passkey.cred_id());
        let json = serde_json::to_string(passkey)
            .map_err(|err| sqlx::Error::Protocol(format!("could not serialise passkey: {err}")))?;

        crate::db_execute!(
            pool,
            "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
             VALUES (?, ?, ?, ?)",
            &id,
            account_id,
            &json,
            label
        )?;

        Ok(id)
    }

    pub async fn list_for_account(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Vec<StoredCredential>, sqlx::Error> {
        crate::db_fetch_all!(
            pool,
            StoredCredential,
            "SELECT id, account_id, passkey_json, label, created_at, last_used_at \
             FROM webauthn_credentials WHERE account_id = ? ORDER BY created_at",
            account_id
        )
    }

    /// Every passkey an account can authenticate with.
    pub async fn passkeys_for_account(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Vec<Passkey>, sqlx::Error> {
        Ok(Self::list_for_account(pool, account_id)
            .await?
            .iter()
            .filter_map(StoredCredential::passkey)
            .collect())
    }

    pub async fn find(
        pool: &DbPool,
        credential_id: &str,
    ) -> Result<Option<StoredCredential>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            StoredCredential,
            "SELECT id, account_id, passkey_json, label, created_at, last_used_at \
             FROM webauthn_credentials WHERE id = ?",
            credential_id
        )
    }

    /// Writes back a credential whose counter moved, and stamps its last use.
    ///
    /// The signature counter is the authenticator's clone-detection signal, so
    /// a successful assertion that advanced it has to be persisted — dropping
    /// the update would silently disable that protection.
    pub async fn record_use(
        pool: &DbPool,
        credential_id: &str,
        passkey: &Passkey,
    ) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(passkey)
            .map_err(|err| sqlx::Error::Protocol(format!("could not serialise passkey: {err}")))?;

        crate::db_execute!(
            pool,
            "UPDATE webauthn_credentials SET passkey_json = ?, last_used_at = ? WHERE id = ?",
            &json,
            Utc::now(),
            credential_id
        )?;

        Ok(())
    }

    pub async fn rename(
        pool: &DbPool,
        account_id: &str,
        credential_id: &str,
        label: &str,
    ) -> Result<bool, sqlx::Error> {
        let affected = crate::db_execute!(
            pool,
            "UPDATE webauthn_credentials SET label = ? WHERE id = ? AND account_id = ?",
            label,
            credential_id,
            account_id
        )?;

        Ok(affected > 0)
    }

    /// How many passkeys an account has.
    pub async fn count_for_account(pool: &DbPool, account_id: &str) -> Result<i64, sqlx::Error> {
        crate::db_scalar_one!(
            pool,
            _,
            "SELECT COUNT(*) FROM webauthn_credentials WHERE account_id = ?",
            account_id
        )
    }

    /// Removes a passkey, refusing to remove the last one.
    ///
    /// Deleting the only credential would leave an account reachable solely
    /// through recovery — a lockout the user performed on themselves, one click
    /// from a settings screen. They must add a replacement first.
    pub async fn delete(
        pool: &DbPool,
        account_id: &str,
        credential_id: &str,
    ) -> Result<DeleteOutcome, sqlx::Error> {
        if Self::count_for_account(pool, account_id).await? <= 1 {
            // If the account has a linked Matrix identity, removing the last passkey
            // does NOT lock the user out: the Matrix account is the primary identity.
            let has_matrix_link = crate::db_scalar_one!(
                pool,
                i64,
                "SELECT COUNT(*) FROM matrix_links WHERE account_id = ?",
                account_id
            )? > 0;

            if !has_matrix_link {
                return Ok(DeleteOutcome::WouldLockOut);
            }
        }

        let affected = crate::db_execute!(
            pool,
            "DELETE FROM webauthn_credentials WHERE id = ? AND account_id = ?",
            credential_id,
            account_id
        )?;

        Ok(if affected > 0 {
            DeleteOutcome::Removed
        } else {
            DeleteOutcome::NotFound
        })
    }
}

pub struct EnrollmentRepository;

impl EnrollmentRepository {
    /// Issues a single-use token to enroll a passkey for `account_id`.
    pub async fn issue(
        pool: &DbPool,
        account_id: &str,
        ttl_minutes: i64,
    ) -> Result<String, sqlx::Error> {
        let token = format!("pk_{}", crate::utils::random::random_hex(20));
        let now = Utc::now();
        let expires_at = now + chrono::Duration::minutes(ttl_minutes);

        crate::db_execute!(
            pool,
            "INSERT INTO passkey_enrollment_tokens (token, account_id, created_at, expires_at) \
             VALUES (?, ?, ?, ?)",
            &token,
            account_id,
            now,
            expires_at
        )?;

        Ok(token)
    }

    /// Validates an enrollment token, marks it consumed, and returns the target account_id.
    ///
    /// Only ever enrolls an account's *first* passkey. A token is minted from
    /// Matrix, and once a passkey exists, letting the Matrix account alone add
    /// another would skip the `!recover` fence entirely — so a token for an
    /// account that has since gained one is refused here, where the capability
    /// is granted, not only where the link was issued.
    pub async fn validate_and_consume(
        pool: &DbPool,
        token: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let now = Utc::now();
        let account_id = crate::db_scalar_optional!(
            pool,
            String,
            "UPDATE passkey_enrollment_tokens \
             SET consumed_at = ? \
             WHERE token = ? AND consumed_at IS NULL AND expires_at > ? \
               AND NOT EXISTS (SELECT 1 FROM webauthn_credentials c \
                               WHERE c.account_id = passkey_enrollment_tokens.account_id) \
             RETURNING account_id",
            now,
            token,
            now
        )?;

        Ok(account_id)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Removed,
    NotFound,
    /// Refused: it was the account's only way in.
    WouldLockOut,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        pool
    }

    /// Inserts a credential row directly. Building a real `Passkey` needs an
    /// authenticator, so the shape of the blob is irrelevant to the ownership
    /// and lockout rules under test here.
    async fn add_raw(pool: &DbPool, id: &str, account_id: &str) {
        crate::db_execute!(
            pool,
            "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
             VALUES (?, ?, '{}', 'test key')",
            id,
            account_id
        )
        .unwrap();
    }

    #[tokio::test]
    async fn test_credentials_are_listed_per_account() {
        let pool = db().await;
        add_raw(&pool, "cred_a1", "acct_a").await;
        add_raw(&pool, "cred_a2", "acct_a").await;
        add_raw(&pool, "cred_b1", "acct_b").await;

        let mine = CredentialRepository::list_for_account(&pool, "acct_a")
            .await
            .unwrap();
        assert_eq!(mine.len(), 2);
        assert!(mine.iter().all(|c| c.account_id == "acct_a"));
    }

    /// Removing the only passkey would lock the account out of itself.
    #[tokio::test]
    async fn test_the_last_passkey_cannot_be_deleted() {
        let pool = db().await;
        add_raw(&pool, "cred_only", "acct_a").await;

        let outcome = CredentialRepository::delete(&pool, "acct_a", "cred_only")
            .await
            .unwrap();
        assert_eq!(outcome, DeleteOutcome::WouldLockOut);
        assert_eq!(
            CredentialRepository::count_for_account(&pool, "acct_a")
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn test_a_passkey_can_be_deleted_once_a_second_exists() {
        let pool = db().await;
        add_raw(&pool, "cred_1", "acct_a").await;
        add_raw(&pool, "cred_2", "acct_a").await;

        let outcome = CredentialRepository::delete(&pool, "acct_a", "cred_1")
            .await
            .unwrap();
        assert_eq!(outcome, DeleteOutcome::Removed);
        assert_eq!(
            CredentialRepository::count_for_account(&pool, "acct_a")
                .await
                .unwrap(),
            1
        );
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

        let outcome = CredentialRepository::delete(&pool, "acct_b", "cred_a1")
            .await
            .unwrap();
        assert_eq!(outcome, DeleteOutcome::NotFound);
        assert_eq!(
            CredentialRepository::count_for_account(&pool, "acct_a")
                .await
                .unwrap(),
            2
        );

        let renamed = CredentialRepository::rename(&pool, "acct_b", "cred_a1", "stolen")
            .await
            .unwrap();
        assert!(
            !renamed,
            "renaming another account's passkey must not succeed"
        );
    }

    /// A credential blob that will not parse must be skipped, not fatal — one
    /// bad row cannot be allowed to lock an account out of its good ones.
    #[tokio::test]
    async fn test_an_unreadable_credential_does_not_break_the_others() {
        let pool = db().await;
        add_raw(&pool, "cred_broken", "acct_a").await;

        let usable = CredentialRepository::passkeys_for_account(&pool, "acct_a")
            .await
            .unwrap();
        assert!(
            usable.is_empty(),
            "the unparseable row is skipped rather than returned"
        );

        // The row is still listed, so the UI can show it and offer removal.
        let listed = CredentialRepository::list_for_account(&pool, "acct_a")
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
    }

    #[tokio::test]
    async fn test_enrollment_token_issue_and_consumption() {
        let pool = db().await;
        let token = EnrollmentRepository::issue(&pool, "acct_matrix", 10)
            .await
            .unwrap();
        assert!(!token.is_empty());

        // First consumption succeeds
        let consumed = EnrollmentRepository::validate_and_consume(&pool, &token)
            .await
            .unwrap();
        assert_eq!(consumed.as_deref(), Some("acct_matrix"));

        // Second consumption fails (single-use)
        let second = EnrollmentRepository::validate_and_consume(&pool, &token)
            .await
            .unwrap();
        assert_eq!(second, None);
    }

    #[tokio::test]
    async fn test_last_passkey_can_be_deleted_if_matrix_linked() {
        let pool = db().await;
        add_raw(&pool, "cred_only", "acct_primary").await;

        // Insert matrix link
        crate::db_execute!(&pool, "INSERT INTO matrix_links (mxid, account_id) VALUES ('@alice:matrix.org', 'acct_primary')")
            .unwrap();

        // Deleting the only passkey succeeds because Matrix is primary identity
        let outcome = CredentialRepository::delete(&pool, "acct_primary", "cred_only")
            .await
            .unwrap();
        assert_eq!(outcome, DeleteOutcome::Removed);
        assert_eq!(
            CredentialRepository::count_for_account(&pool, "acct_primary")
                .await
                .unwrap(),
            0
        );
    }
}
