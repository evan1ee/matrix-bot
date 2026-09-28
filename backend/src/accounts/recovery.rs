//! Getting back in after losing every passkey.
//!
//! Two independent routes, so neither single loss is fatal:
//!
//! * **Recovery codes** — ten of them, shown once at registration, each usable
//!   once. Presenting one signs you straight in.
//! * **Matrix recovery** — `!recover` from a linked Matrix account. This one is
//!   fenced, because it creates a circular trust: linking a Matrix account
//!   requires a recent passkey, and passkey recovery would otherwise require
//!   only the Matrix account. Whoever controls that account would control the
//!   local one outright.
//!
//! The fence has three parts. The request is announced to every linked identity
//! *immediately* and does nothing for a day, during which any live session can
//! cancel it — so a compromised Matrix account buys an attacker 24 hours of
//! loud warnings rather than instant access. When it does mature it grants a
//! **restricted** session: one that can register a passkey and nothing else for
//! a further 72 hours. An attacker who waits out the delay still cannot unlink
//! the real owner's identities, remove their passkeys, or export their
//! finances.

use crate::db::DbPool;
use chrono::{Duration, Utc};

/// How many codes are issued at once.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// How long a Matrix-initiated recovery waits before it can be used.
pub const RECOVERY_DELAY_HOURS: i64 = 24;

/// How long the matured code stays usable.
pub const RECOVERY_WINDOW_HOURS: i64 = 48;

/// How long a session created by recovery stays restricted.
pub const RECOVERY_RESTRICTION_HOURS: i64 = 72;

/// Work factor for hashing a recovery code.
///
/// Lower than bcrypt's default of 12, and deliberately so. A bcrypt cost defends a
/// *guessable* secret by making each guess expensive; these codes are ten
/// characters drawn by a CSPRNG from a 31-symbol alphabet — around 50 bits —
/// so guessing is already out of reach and the work factor is not what stands
/// between an attacker and the account. Hashing is here so a leaked backup
/// hands over nothing usable, which any cost achieves.

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Sign the holder in.
    Accepted,
    /// No matching unused code.
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MaturedRecovery {
    /// Ready: open a restricted session.
    Ready { account_id: String },
    /// Still inside the 24-hour delay.
    TooEarly,
    /// Cancelled, expired, already used, or never existed.
    Unavailable,
}

pub struct RecoveryRepository;

impl RecoveryRepository {
    /// Issues a fresh set of codes, invalidating any previous set.
    pub async fn issue_codes(pool: &DbPool, account_id: &str) -> Result<Vec<String>, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::recovery::issue_codes(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// How many codes are still unused.
    pub async fn remaining(pool: &DbPool, account_id: &str) -> Result<i64, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::recovery::remaining(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// Checks a code against every account.
    pub async fn redeem(pool: &DbPool, code: &str) -> Result<Option<String>, sqlx::Error> {
        let res =
            crate::infrastructure::postgres::repositories::accounts::recovery::redeem(pool, code)
                .await?;
        Ok(res.map(|a| a.into_inner()))
    }

    /// Starts a Matrix-initiated recovery.
    pub async fn request(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<(String, chrono::DateTime<Utc>), sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::recovery::request(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// Cancels every live recovery request for an account.
    pub async fn cancel_all(pool: &DbPool, account_id: &str) -> Result<u64, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::recovery::cancel_all(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// Whether an account has a recovery request currently counting down.
    pub async fn pending(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Option<chrono::DateTime<Utc>>, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::recovery::pending(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    /// Redeems a matured recovery code.
    pub async fn redeem_request(pool: &DbPool, code: &str) -> Result<MaturedRecovery, sqlx::Error> {
        let res =
            crate::infrastructure::postgres::repositories::accounts::recovery::redeem_request(
                pool, code,
            )
            .await?;
        Ok(match res {
            crate::domain::accounts::MaturedRecovery::Ready { account_id } => {
                MaturedRecovery::Ready {
                    account_id: account_id.into_inner(),
                }
            }
            crate::domain::accounts::MaturedRecovery::TooEarly => MaturedRecovery::TooEarly,
            crate::domain::accounts::MaturedRecovery::Unavailable => MaturedRecovery::Unavailable,
        })
    }

    /// When a session opened by recovery stops being restricted.
    pub fn restriction_expiry() -> chrono::DateTime<Utc> {
        Utc::now() + Duration::hours(RECOVERY_RESTRICTION_HOURS)
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

    async fn account(pool: &DbPool, name: &str) -> String {
        AccountRepository::create(pool, name, STATUS_ACTIVE)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_codes_are_issued_hashed_and_unique() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;

        let codes = RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();
        assert_eq!(codes.len(), RECOVERY_CODE_COUNT);

        let unique: std::collections::HashSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), RECOVERY_CODE_COUNT, "codes must not repeat");

        // Nothing recoverable is stored.
        let stored: Vec<String> = sqlx::query_scalar!("SELECT code_hash FROM recovery_codes")
            .fetch_all(&pool)
            .await
            .unwrap();
        for code in &codes {
            assert!(
                !stored.iter().any(|h| h.contains(code)),
                "a code is stored in the clear"
            );
        }
    }

    #[tokio::test]
    async fn test_a_code_signs_you_in_once() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let codes = RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(
            RecoveryRepository::redeem(&pool, &codes[0]).await.unwrap(),
            Some(acct.clone())
        );
        assert_eq!(
            RecoveryRepository::redeem(&pool, &codes[0]).await.unwrap(),
            None,
            "a used code must not work twice"
        );
        assert_eq!(
            RecoveryRepository::remaining(&pool, &acct).await.unwrap(),
            9
        );
    }

    #[tokio::test]
    async fn test_a_wrong_code_is_rejected() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(
            RecoveryRepository::redeem(&pool, "ZZZZZ-ZZZZZ")
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            RecoveryRepository::remaining(&pool, &acct).await.unwrap(),
            10
        );
    }

    /// Reissuing must invalidate the previous set, or a printout someone threw
    /// away stays live.
    #[tokio::test]
    async fn test_reissuing_invalidates_the_old_set() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let first = RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();
        let second = RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(
            RecoveryRepository::redeem(&pool, &first[0]).await.unwrap(),
            None
        );
        assert_eq!(
            RecoveryRepository::redeem(&pool, &second[0]).await.unwrap(),
            Some(acct)
        );
    }

    /// One account's codes must never open another's.
    #[tokio::test]
    async fn test_a_code_only_opens_its_own_account() {
        let pool = db().await;
        let mine = account(&pool, "mine").await;
        let theirs = account(&pool, "theirs").await;

        let my_codes = RecoveryRepository::issue_codes(&pool, &mine).await.unwrap();
        RecoveryRepository::issue_codes(&pool, &theirs)
            .await
            .unwrap();

        assert_eq!(
            RecoveryRepository::redeem(&pool, &my_codes[0])
                .await
                .unwrap(),
            Some(mine)
        );
    }

    /// The delay is the whole defence when the Matrix account is what was
    /// compromised. A code that worked immediately would make a linked Matrix
    /// account a straight backdoor.
    #[tokio::test]
    async fn test_matrix_recovery_does_nothing_for_a_day() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;

        let (code, valid_after) = RecoveryRepository::request(&pool, &acct).await.unwrap();
        assert!(valid_after > Utc::now() + Duration::hours(RECOVERY_DELAY_HOURS - 1));

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code)
                .await
                .unwrap(),
            MaturedRecovery::TooEarly
        );

        // The countdown is visible, so the UI can say what is happening.
        assert!(RecoveryRepository::pending(&pool, &acct)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_a_matured_request_opens_the_account() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();

        // Wind the clock forward.
        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
            Utc::now() - Duration::minutes(1),
            &acct
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code)
                .await
                .unwrap(),
            MaturedRecovery::Ready {
                account_id: acct.clone()
            }
        );

        // And is single-use.
        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code)
                .await
                .unwrap(),
            MaturedRecovery::Unavailable
        );
    }

    /// The real owner, still signed in, sees the notification and stops it.
    /// This is what the 24-hour delay exists to make possible.
    #[tokio::test]
    async fn test_a_live_session_can_cancel_a_recovery() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();

        assert_eq!(
            RecoveryRepository::cancel_all(&pool, &acct).await.unwrap(),
            1
        );
        assert!(RecoveryRepository::pending(&pool, &acct)
            .await
            .unwrap()
            .is_none());

        // Even once it would have matured, it is dead.
        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
            Utc::now() - Duration::minutes(1),
            &acct
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code)
                .await
                .unwrap(),
            MaturedRecovery::Unavailable
        );
    }

    /// Repeated `!recover` must not let an attacker bury the warning under a
    /// pile of requests, each with its own live code.
    #[tokio::test]
    async fn test_only_one_recovery_request_is_live_at_a_time() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;

        let (first, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();
        let (second, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();

        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
            Utc::now() - Duration::minutes(1),
            &acct
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &first)
                .await
                .unwrap(),
            MaturedRecovery::Unavailable,
            "the superseded request must be dead"
        );
        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &second)
                .await
                .unwrap(),
            MaturedRecovery::Ready { account_id: acct }
        );
    }

    /// An expired request cannot be redeemed even though it matured.
    #[tokio::test]
    async fn test_a_matured_request_still_expires() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();

        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1, expires_at = $2 WHERE account_id = $3",
            Utc::now() - Duration::hours(2),
            Utc::now() - Duration::hours(1),
            &acct
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code)
                .await
                .unwrap(),
            MaturedRecovery::Unavailable
        );
    }
}
