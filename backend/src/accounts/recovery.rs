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

use bcrypt::{hash, verify};
use chrono::{Duration, Utc};
use sqlx::SqlitePool;

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
///
/// It is not zero, either: `redeem` compares against every unused hash in the
/// table, so the cost is paid once per candidate on a legitimate redemption.
const RECOVERY_HASH_COST: u32 = 8;

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
    ///
    /// Returns the plaintext, which is the only time it exists — the caller
    /// must show it once and never store it.
    pub async fn issue_codes(
        pool: &SqlitePool,
        account_id: &str,
    ) -> Result<Vec<String>, sqlx::Error> {
        sqlx::query("DELETE FROM recovery_codes WHERE account_id = ?")
            .bind(account_id)
            .execute(pool)
            .await?;

        let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
        for _ in 0..RECOVERY_CODE_COUNT {
            let code = random_code(pool).await?;
            let hashed = hash(&code, RECOVERY_HASH_COST)
                .map_err(|err| sqlx::Error::Protocol(format!("could not hash a code: {err}")))?;

            sqlx::query("INSERT INTO recovery_codes (account_id, code_hash) VALUES (?, ?)")
                .bind(account_id)
                .bind(&hashed)
                .execute(pool)
                .await?;

            codes.push(code);
        }

        Ok(codes)
    }

    /// How many codes are still unused. Shown in the UI so someone can tell
    /// they are running low before they run out.
    pub async fn remaining(pool: &SqlitePool, account_id: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM recovery_codes WHERE account_id = ? AND used_at IS NULL",
        )
        .bind(account_id)
        .fetch_one(pool)
        .await
    }

    /// Checks a code against every account, since the user has no way to say
    /// who they are — the code *is* the claim.
    ///
    /// Every unused hash is compared. There is no index to narrow this by, and
    /// deliberately so: a lookup keyed on anything derivable from the code
    /// would leak which codes exist.
    pub async fn redeem(
        pool: &SqlitePool,
        code: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let normalised = normalise(code);

        let candidates = sqlx::query_as::<_, (i64, String, String)>(
            "SELECT id, account_id, code_hash FROM recovery_codes WHERE used_at IS NULL",
        )
        .fetch_all(pool)
        .await?;

        for (id, account_id, code_hash) in candidates {
            if verify(&normalised, &code_hash).unwrap_or(false) {
                let consumed = sqlx::query(
                    "UPDATE recovery_codes SET used_at = ? WHERE id = ? AND used_at IS NULL",
                )
                .bind(Utc::now())
                .bind(id)
                .execute(pool)
                .await?
                .rows_affected();

                // Lost a race with a concurrent redemption of the same code.
                if consumed == 0 {
                    return Ok(None);
                }

                return Ok(Some(account_id));
            }
        }

        Ok(None)
    }

    /// Starts a Matrix-initiated recovery.
    ///
    /// Returns the code to DM to the user *now* — but it does not work until
    /// `valid_after`. Telling them the code immediately is deliberate: the
    /// message is also the warning, and it should arrive the moment the request
    /// is made rather than a day later.
    pub async fn request(
        pool: &SqlitePool,
        account_id: &str,
    ) -> Result<(String, chrono::DateTime<Utc>), sqlx::Error> {
        // One live request at a time, so repeated `!recover` cannot be used to
        // paper over the notification with noise.
        sqlx::query(
            "UPDATE recovery_requests SET cancelled_at = ? \
             WHERE account_id = ? AND cancelled_at IS NULL AND consumed_at IS NULL",
        )
        .bind(Utc::now())
        .bind(account_id)
        .execute(pool)
        .await?;

        let code = random_code(pool).await?;
        let hashed = hash(&code, RECOVERY_HASH_COST)
            .map_err(|err| sqlx::Error::Protocol(format!("could not hash a code: {err}")))?;

        let now = Utc::now();
        let valid_after = now + Duration::hours(RECOVERY_DELAY_HOURS);

        let id = sqlx::query_scalar::<_, String>(
            "INSERT INTO recovery_requests \
             (id, account_id, code_hash, requested_at, valid_after, expires_at) \
             VALUES (lower(hex(randomblob(16))), ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(account_id)
        .bind(&hashed)
        .bind(now)
        .bind(valid_after)
        .bind(valid_after + Duration::hours(RECOVERY_WINDOW_HOURS))
        .fetch_one(pool)
        .await?;

        tracing::info!("Recovery requested for {account_id} (request {id})");

        Ok((code, valid_after))
    }

    /// Cancels every live recovery request for an account.
    ///
    /// This is what makes the delay meaningful: the real owner, still holding a
    /// working session, sees the notification and stops it.
    pub async fn cancel_all(pool: &SqlitePool, account_id: &str) -> Result<u64, sqlx::Error> {
        Ok(sqlx::query(
            "UPDATE recovery_requests SET cancelled_at = ? \
             WHERE account_id = ? AND cancelled_at IS NULL AND consumed_at IS NULL",
        )
        .bind(Utc::now())
        .bind(account_id)
        .execute(pool)
        .await?
        .rows_affected())
    }

    /// Whether an account has a recovery request currently counting down.
    pub async fn pending(
        pool: &SqlitePool,
        account_id: &str,
    ) -> Result<Option<chrono::DateTime<Utc>>, sqlx::Error> {
        sqlx::query_scalar::<_, chrono::DateTime<Utc>>(
            "SELECT valid_after FROM recovery_requests \
             WHERE account_id = ? AND cancelled_at IS NULL AND consumed_at IS NULL \
               AND expires_at > ? ORDER BY requested_at DESC LIMIT 1",
        )
        .bind(account_id)
        .bind(Utc::now())
        .fetch_optional(pool)
        .await
    }

    /// Redeems a matured recovery code.
    pub async fn redeem_request(
        pool: &SqlitePool,
        code: &str,
    ) -> Result<MaturedRecovery, sqlx::Error> {
        let normalised = normalise(code);
        let now = Utc::now();

        let candidates = sqlx::query_as::<_, (String, String, String, chrono::DateTime<Utc>)>(
            "SELECT id, account_id, code_hash, valid_after FROM recovery_requests \
             WHERE cancelled_at IS NULL AND consumed_at IS NULL AND expires_at > ?",
        )
        .bind(now)
        .fetch_all(pool)
        .await?;

        for (id, account_id, code_hash, valid_after) in candidates {
            if !verify(&normalised, &code_hash).unwrap_or(false) {
                continue;
            }

            if now < valid_after {
                return Ok(MaturedRecovery::TooEarly);
            }

            let consumed = sqlx::query(
                "UPDATE recovery_requests SET consumed_at = ? WHERE id = ? AND consumed_at IS NULL",
            )
            .bind(now)
            .bind(&id)
            .execute(pool)
            .await?
            .rows_affected();

            return Ok(if consumed == 1 {
                MaturedRecovery::Ready { account_id }
            } else {
                MaturedRecovery::Unavailable
            });
        }

        Ok(MaturedRecovery::Unavailable)
    }

    /// When a session opened by recovery stops being restricted.
    pub fn restriction_expiry() -> chrono::DateTime<Utc> {
        Utc::now() + Duration::hours(RECOVERY_RESTRICTION_HOURS)
    }
}

/// A code a person can read off a screen and type into a chat window.
async fn random_code(pool: &SqlitePool) -> Result<String, sqlx::Error> {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

    let bytes: Vec<u8> = sqlx::query_scalar::<_, Vec<u8>>("SELECT randomblob(10)")
        .fetch_one(pool)
        .await?;

    let raw: String = bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();

    Ok(format!("{}-{}", &raw[..5], &raw[5..10]))
}

fn normalise(code: &str) -> String {
    let cleaned: String = code
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();

    if cleaned.len() == 10 {
        format!("{}-{}", &cleaned[..5], &cleaned[5..])
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use crate::db::schema::run_migrations;

    async fn db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn account(pool: &SqlitePool, name: &str) -> String {
        AccountRepository::create(pool, name, STATUS_ACTIVE).await.unwrap()
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
        let stored: Vec<String> = sqlx::query_scalar("SELECT code_hash FROM recovery_codes")
            .fetch_all(&pool)
            .await
            .unwrap();
        for code in &codes {
            assert!(!stored.iter().any(|h| h.contains(code)), "a code is stored in the clear");
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
        assert_eq!(RecoveryRepository::remaining(&pool, &acct).await.unwrap(), 9);
    }

    #[tokio::test]
    async fn test_a_wrong_code_is_rejected() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(RecoveryRepository::redeem(&pool, "ZZZZZ-ZZZZZ").await.unwrap(), None);
        assert_eq!(RecoveryRepository::remaining(&pool, &acct).await.unwrap(), 10);
    }

    /// Reissuing must invalidate the previous set, or a printout someone threw
    /// away stays live.
    #[tokio::test]
    async fn test_reissuing_invalidates_the_old_set() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let first = RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();
        let second = RecoveryRepository::issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(RecoveryRepository::redeem(&pool, &first[0]).await.unwrap(), None);
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
        RecoveryRepository::issue_codes(&pool, &theirs).await.unwrap();

        assert_eq!(
            RecoveryRepository::redeem(&pool, &my_codes[0]).await.unwrap(),
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
            RecoveryRepository::redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::TooEarly
        );

        // The countdown is visible, so the UI can say what is happening.
        assert!(RecoveryRepository::pending(&pool, &acct).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_a_matured_request_opens_the_account() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();

        // Wind the clock forward.
        sqlx::query("UPDATE recovery_requests SET valid_after = ? WHERE account_id = ?")
            .bind(Utc::now() - Duration::minutes(1))
            .bind(&acct)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::Ready { account_id: acct.clone() }
        );

        // And is single-use.
        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code).await.unwrap(),
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

        assert_eq!(RecoveryRepository::cancel_all(&pool, &acct).await.unwrap(), 1);
        assert!(RecoveryRepository::pending(&pool, &acct).await.unwrap().is_none());

        // Even once it would have matured, it is dead.
        sqlx::query("UPDATE recovery_requests SET valid_after = ? WHERE account_id = ?")
            .bind(Utc::now() - Duration::minutes(1))
            .bind(&acct)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code).await.unwrap(),
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

        sqlx::query("UPDATE recovery_requests SET valid_after = ? WHERE account_id = ?")
            .bind(Utc::now() - Duration::minutes(1))
            .bind(&acct)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &first).await.unwrap(),
            MaturedRecovery::Unavailable,
            "the superseded request must be dead"
        );
        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &second).await.unwrap(),
            MaturedRecovery::Ready { account_id: acct }
        );
    }

    /// An expired request cannot be redeemed even though it matured.
    #[tokio::test]
    async fn test_a_matured_request_still_expires() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = RecoveryRepository::request(&pool, &acct).await.unwrap();

        sqlx::query("UPDATE recovery_requests SET valid_after = ?, expires_at = ? WHERE account_id = ?")
            .bind(Utc::now() - Duration::hours(2))
            .bind(Utc::now() - Duration::hours(1))
            .bind(&acct)
            .execute(&pool)
            .await
            .unwrap();

        assert_eq!(
            RecoveryRepository::redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::Unavailable
        );
    }
}
