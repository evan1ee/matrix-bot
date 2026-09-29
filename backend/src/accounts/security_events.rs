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

use crate::db::DbPool;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

pub use crate::domain::accounts::SecurityEventKind;

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
        pool: &DbPool,
        account_id: impl Into<crate::domain::accounts::AccountId>,
        kind: SecurityEventKind,
        detail: Option<&str>,
    ) {
        let id = account_id.into();
        crate::infrastructure::postgres::repositories::accounts::security_events::record(
            pool, &id, kind, detail,
        )
        .await;
    }

    /// The account's recent history, newest first.
    pub async fn list(
        pool: &DbPool,
        account_id: impl Into<crate::domain::accounts::AccountId>,
        limit: i64,
    ) -> Result<Vec<SecurityEvent>, sqlx::Error> {
        let id = account_id.into();
        let events =
            crate::infrastructure::postgres::repositories::accounts::security_events::list(
                pool, &id, limit,
            )
            .await?;
        Ok(events
            .into_iter()
            .map(|e| SecurityEvent {
                id: e.id,
                kind: e.kind,
                detail: e.detail,
                created_at: e.created_at,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
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
        let pool = crate::db::pool::test_pool().await;
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
