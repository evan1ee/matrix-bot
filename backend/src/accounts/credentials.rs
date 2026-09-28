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

impl From<crate::domain::accounts::StoredCredential> for StoredCredential {
    fn from(s: crate::domain::accounts::StoredCredential) -> Self {
        Self {
            id: s.id,
            account_id: s.account_id.into_inner(),
            passkey_json: s.passkey_json,
            label: s.label,
            created_at: s.created_at,
            last_used_at: s.last_used_at,
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

        crate::infrastructure::postgres::repositories::accounts::credentials::add(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
            &id,
            &json,
            label,
        )
        .await?;

        Ok(id)
    }

    pub async fn list_for_account(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Vec<StoredCredential>, sqlx::Error> {
        let creds =
            crate::infrastructure::postgres::repositories::accounts::credentials::list_for_account(
                pool,
                &crate::domain::accounts::AccountId::new(account_id),
            )
            .await?;
        Ok(creds.into_iter().map(Into::into).collect())
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
        let cred = crate::infrastructure::postgres::repositories::accounts::credentials::find(
            pool,
            credential_id,
        )
        .await?;
        Ok(cred.map(Into::into))
    }

    /// Writes back a credential whose counter moved, and stamps its last use.
    pub async fn record_use(
        pool: &DbPool,
        credential_id: &str,
        passkey: &Passkey,
    ) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(passkey)
            .map_err(|err| sqlx::Error::Protocol(format!("could not serialise passkey: {err}")))?;

        crate::infrastructure::postgres::repositories::accounts::credentials::record_use(
            pool,
            credential_id,
            &json,
        )
        .await
    }

    pub async fn rename(
        pool: &DbPool,
        account_id: &str,
        credential_id: &str,
        label: &str,
    ) -> Result<bool, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::credentials::rename(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
            credential_id,
            label,
        )
        .await
    }

    /// How many passkeys an account has.
    pub async fn count_for_account(pool: &DbPool, account_id: &str) -> Result<i64, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::credentials::count_for_account(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// Removes a passkey, refusing to remove the last one.
    pub async fn delete(
        pool: &DbPool,
        account_id: &str,
        credential_id: &str,
    ) -> Result<DeleteOutcome, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::credentials::delete(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
            credential_id,
        )
        .await
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
        crate::infrastructure::postgres::repositories::accounts::credentials::issue_enrollment_token(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
            ttl_minutes,
        )
        .await
    }

    /// Validates an enrollment token, marks it consumed, and returns the target account_id.
    pub async fn validate_and_consume(
        pool: &DbPool,
        token: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let res = crate::infrastructure::postgres::repositories::accounts::credentials::validate_and_consume_enrollment_token(
            pool,
            token,
        )
        .await?;
        Ok(res.map(|a| a.into_inner()))
    }
}

pub use crate::domain::accounts::DeleteOutcome;

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    /// Inserts a credential row directly. Building a real `Passkey` needs an
    /// authenticator, so the shape of the blob is irrelevant to the ownership
    /// and lockout rules under test here.
    async fn add_raw(pool: &DbPool, id: &str, account_id: &str) {
        sqlx::query!(
            "INSERT INTO webauthn_credentials (id, account_id, passkey_json, label) \
             VALUES ($1, $2, '{}', 'test key')",
            id,
            account_id
        )
        .execute(pool)
        .await
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
        sqlx::query!(
            "INSERT INTO matrix_links (mxid, account_id) VALUES ('@alice:matrix.org', 'acct_primary')"
        )
        .execute(&pool)
        .await
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
