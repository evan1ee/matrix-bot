//! The account's own audit log, and the notifications that go with it.
//!
//! Every sensitive action is recorded here *and* announced by DM to each linked
//! Matrix identity. The two halves cover each other: an account with no Matrix
//! link still has somewhere to see what happened, and a user who never opens
//! the web UI still hears about it.
//!
//! Recording must never be the reason an action fails. If the log write or the
//! DM fails, the action it describes has already happened — refusing it after
//! the fact would be worse than a missing line in a list.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::SqlitePool;
use utoipa::ToSchema;

/// What happened. Stored verbatim, so a new variant needs no migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityEventKind {
    SignedIn,
    PasskeyAdded,
    PasskeyRemoved,
    PasskeyRenamed,
    MatrixLinked,
    MatrixUnlinked,
    AccountsMerged,
    RecoveryCodesIssued,
    RecoveryCodeUsed,
    RecoveryRequested,
    RecoveryCancelled,
    RecoveryCompleted,
    SessionsRevoked,
    FinancialDataExported,
}

impl SecurityEventKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SignedIn => "signed_in",
            Self::PasskeyAdded => "passkey_added",
            Self::PasskeyRemoved => "passkey_removed",
            Self::PasskeyRenamed => "passkey_renamed",
            Self::MatrixLinked => "matrix_linked",
            Self::MatrixUnlinked => "matrix_unlinked",
            Self::AccountsMerged => "accounts_merged",
            Self::RecoveryCodesIssued => "recovery_codes_issued",
            Self::RecoveryCodeUsed => "recovery_code_used",
            Self::RecoveryRequested => "recovery_requested",
            Self::RecoveryCancelled => "recovery_cancelled",
            Self::RecoveryCompleted => "recovery_completed",
            Self::SessionsRevoked => "sessions_revoked",
            Self::FinancialDataExported => "financial_data_exported",
        }
    }

    /// Whether this is worth interrupting someone's day over.
    ///
    /// Signing in happens constantly and DMing about it would train the user to
    /// ignore the channel — which is exactly the channel a recovery warning has
    /// to arrive on and be read. Everything that changes *how the account can
    /// be reached* is notified.
    pub fn warrants_a_dm(&self) -> bool {
        !matches!(self, Self::SignedIn)
    }

    /// One line, addressed to the person whose account it is.
    pub fn message(&self, detail: Option<&str>) -> String {
        let suffix = detail.map(|d| format!(" ({d})")).unwrap_or_default();
        match self {
            Self::SignedIn => format!("Signed in{suffix}."),
            Self::PasskeyAdded => format!("A passkey was added to your account{suffix}."),
            Self::PasskeyRemoved => format!("A passkey was removed from your account{suffix}."),
            Self::PasskeyRenamed => format!("A passkey was renamed{suffix}."),
            Self::MatrixLinked => format!("A Matrix account was linked{suffix}."),
            Self::MatrixUnlinked => format!("A Matrix account was unlinked{suffix}."),
            Self::AccountsMerged => format!("Existing data was merged into your account{suffix}."),
            Self::RecoveryCodesIssued => {
                format!("New recovery codes were generated{suffix}. Your old codes no longer work.")
            }
            Self::RecoveryCodeUsed => {
                format!("A recovery code was used to sign in{suffix}.")
            }
            Self::RecoveryRequested => format!(
                "Account recovery was requested{suffix}. It will not take effect for 24 hours — \
                 if this was not you, cancel it from any signed-in device now."
            ),
            Self::RecoveryCancelled => format!("Account recovery was cancelled{suffix}."),
            Self::RecoveryCompleted => format!("Account recovery completed{suffix}."),
            Self::SessionsRevoked => format!("Other sessions were signed out{suffix}."),
            Self::FinancialDataExported => format!("Financial data was exported{suffix}."),
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema, sqlx::FromRow)]
pub struct SecurityEvent {
    pub id: i64,
    pub kind: String,
    pub detail: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

pub struct SecurityEventRepository;

impl SecurityEventRepository {
    /// Records an event. Errors are logged and swallowed — see the module note.
    pub async fn record(
        pool: &SqlitePool,
        account_id: &str,
        kind: SecurityEventKind,
        detail: Option<&str>,
    ) {
        let result =
            sqlx::query("INSERT INTO security_events (account_id, kind, detail) VALUES (?, ?, ?)")
                .bind(account_id)
                .bind(kind.as_str())
                .bind(detail)
                .execute(pool)
                .await;

        if let Err(err) = result {
            tracing::error!(
                "Could not record security event {} for {}: {:?}",
                kind.as_str(),
                account_id,
                err
            );
        }
    }

    /// The account's recent history, newest first.
    pub async fn list(
        pool: &SqlitePool,
        account_id: &str,
        limit: i64,
    ) -> Result<Vec<SecurityEvent>, sqlx::Error> {
        sqlx::query_as::<_, SecurityEvent>(
            "SELECT id, kind, detail, created_at FROM security_events \
             WHERE account_id = ? ORDER BY created_at DESC, id DESC LIMIT ?",
        )
        .bind(account_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use crate::db::schema::run_migrations;

    async fn db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&crate::db::DbPool::Sqlite(pool.clone()))
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_events_are_recorded_and_listed_newest_first() {
        let pool = db().await;
        let acct = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();

        SecurityEventRepository::record(&pool, &acct, SecurityEventKind::SignedIn, None).await;
        SecurityEventRepository::record(
            &pool,
            &acct,
            SecurityEventKind::MatrixLinked,
            Some("@evan:mbm.one"),
        )
        .await;

        let events = SecurityEventRepository::list(&pool, &acct, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "matrix_linked");
        assert_eq!(events[0].detail.as_deref(), Some("@evan:mbm.one"));
    }

    /// One account's log must never show another's.
    #[tokio::test]
    async fn test_the_log_is_per_account() {
        let pool = db().await;
        let mine = AccountRepository::create(&pool, "mine", STATUS_ACTIVE)
            .await
            .unwrap();
        let theirs = AccountRepository::create(&pool, "theirs", STATUS_ACTIVE)
            .await
            .unwrap();

        SecurityEventRepository::record(&pool, &theirs, SecurityEventKind::SignedIn, None).await;

        assert!(SecurityEventRepository::list(&pool, &mine, 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            SecurityEventRepository::list(&pool, &theirs, 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    /// Recording must not be able to fail an action that already happened.
    #[tokio::test]
    async fn test_recording_against_a_broken_table_does_not_panic() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        // No migrations: the table does not exist.
        SecurityEventRepository::record(&pool, "acct_a", SecurityEventKind::SignedIn, None).await;
    }

    /// Routine sign-ins must not DM, or the channel becomes noise and the one
    /// message that matters — a recovery warning — gets ignored with the rest.
    #[test]
    fn test_only_meaningful_events_are_notified() {
        assert!(!SecurityEventKind::SignedIn.warrants_a_dm());

        for kind in [
            SecurityEventKind::PasskeyRemoved,
            SecurityEventKind::MatrixUnlinked,
            SecurityEventKind::RecoveryRequested,
            SecurityEventKind::FinancialDataExported,
        ] {
            assert!(kind.warrants_a_dm(), "{} must be announced", kind.as_str());
        }
    }

    /// The recovery warning has to tell someone what to do about it, because it
    /// is the only thing standing between a stolen Matrix account and a stolen
    /// local one.
    #[test]
    fn test_the_recovery_warning_says_how_to_stop_it() {
        let message = SecurityEventKind::RecoveryRequested.message(None);
        assert!(message.contains("24 hours"));
        assert!(message.contains("cancel"));
    }

    #[test]
    fn test_details_are_included_when_present() {
        let with = SecurityEventKind::MatrixLinked.message(Some("@evan:mbm.one"));
        assert!(with.contains("@evan:mbm.one"));

        let without = SecurityEventKind::MatrixLinked.message(None);
        assert!(!without.contains('('));
    }
}
