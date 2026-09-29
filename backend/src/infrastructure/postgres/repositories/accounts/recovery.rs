//! Account recovery repository: recovery codes and Matrix-initiated recovery requests.

use bcrypt::{hash, verify};
use chrono::{DateTime, Duration, Utc};

use crate::domain::accounts::{
    AccountId, MaturedRecovery, RECOVERY_CODE_COUNT, RECOVERY_DELAY_HOURS, RECOVERY_WINDOW_HOURS,
};

const RECOVERY_HASH_COST: u32 = 8;

pub async fn issue_codes(
    pool: &sqlx::PgPool,
    account_id: &AccountId,
) -> Result<Vec<String>, sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query!(
        "DELETE FROM recovery_codes WHERE account_id = $1",
        account_id.as_uuid(),
    )
    .execute(&mut *tx)
    .await?;

    let mut codes = Vec::with_capacity(RECOVERY_CODE_COUNT);
    for _ in 0..RECOVERY_CODE_COUNT {
        let code = random_code();
        let hashed = hash(&code, RECOVERY_HASH_COST)
            .map_err(|err| sqlx::Error::Protocol(format!("could not hash a code: {err}")))?;

        sqlx::query!(
            "INSERT INTO recovery_codes (account_id, code_hash) VALUES ($1, $2)",
            account_id.as_uuid(),
            &hashed,
        )
        .execute(&mut *tx)
        .await?;

        codes.push(code);
    }

    tx.commit().await?;

    Ok(codes)
}

#[cfg(test)]
pub async fn set_valid_after(
    pool: &sqlx::PgPool,
    account_id: &AccountId,
    valid_after: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
        valid_after,
        account_id.as_uuid(),
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remaining(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        r#"SELECT COUNT(*) AS "count!" FROM recovery_codes WHERE account_id = $1 AND used_at IS NULL"#,
        account_id.as_uuid(),
    )
    .fetch_one(conn)
    .await?;
    Ok(count)
}

pub async fn redeem(pool: &sqlx::PgPool, code: &str) -> Result<Option<AccountId>, sqlx::Error> {
    let normalised = normalise(code);

    let candidates =
        sqlx::query!(r#"SELECT id, account_id AS "account_id: uuid::Uuid", code_hash FROM recovery_codes WHERE used_at IS NULL"#)
            .fetch_all(pool)
            .await?;

    for row in candidates {
        if verify(&normalised, &row.code_hash).unwrap_or(false) {
            let consumed = sqlx::query!(
                "UPDATE recovery_codes SET used_at = $1 WHERE id = $2 AND used_at IS NULL",
                Utc::now(),
                row.id,
            )
            .execute(pool)
            .await?
            .rows_affected();

            if consumed == 0 {
                return Ok(None);
            }

            return Ok(Some(AccountId::from_uuid(row.account_id)));
        }
    }

    Ok(None)
}

pub async fn request(
    pool: &sqlx::PgPool,
    account_id: &AccountId,
) -> Result<(String, DateTime<Utc>), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query!(
        "UPDATE recovery_requests SET cancelled_at = $1 \
         WHERE account_id = $2 AND cancelled_at IS NULL AND consumed_at IS NULL",
        Utc::now(),
        account_id.as_uuid(),
    )
    .execute(&mut *tx)
    .await?;

    let code = random_code();
    let hashed = hash(&code, RECOVERY_HASH_COST)
        .map_err(|err| sqlx::Error::Protocol(format!("could not hash a code: {err}")))?;

    let now = Utc::now();
    let valid_after = now + Duration::hours(RECOVERY_DELAY_HOURS);
    let expires = valid_after + Duration::hours(RECOVERY_WINDOW_HOURS);
    let id = crate::utils::random::random_hex(16);

    sqlx::query!(
        "INSERT INTO recovery_requests \
         (id, account_id, code_hash, requested_at, valid_after, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
        id,
        account_id.as_uuid(),
        &hashed,
        now,
        valid_after,
        expires,
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    tracing::info!(
        account_id = %account_id,
        request_id = %id,
        "Recovery requested"
    );

    Ok((code, valid_after))
}

pub async fn cancel_all(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<u64, sqlx::Error> {
    let affected = sqlx::query!(
        "UPDATE recovery_requests SET cancelled_at = $1 \
         WHERE account_id = $2 AND cancelled_at IS NULL AND consumed_at IS NULL",
        Utc::now(),
        account_id.as_uuid(),
    )
    .execute(conn)
    .await?
    .rows_affected();
    Ok(affected)
}

pub async fn pending(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Option<DateTime<Utc>>, sqlx::Error> {
    let row = sqlx::query_scalar!(
        "SELECT valid_after FROM recovery_requests \
         WHERE account_id = $1 AND cancelled_at IS NULL AND consumed_at IS NULL \
           AND expires_at > $2 ORDER BY requested_at DESC LIMIT 1",
        account_id.as_uuid(),
        Utc::now(),
    )
    .fetch_optional(conn)
    .await?;
    Ok(row)
}

pub async fn redeem_request(
    pool: &sqlx::PgPool,
    code: &str,
) -> Result<MaturedRecovery, sqlx::Error> {
    let normalised = normalise(code);
    let now = Utc::now();

    let candidates = sqlx::query!(
        r#"SELECT id, account_id AS "account_id: uuid::Uuid", code_hash, valid_after FROM recovery_requests
         WHERE cancelled_at IS NULL AND consumed_at IS NULL AND expires_at > $1"#,
        now,
    )
    .fetch_all(pool)
    .await?;

    for row in candidates {
        if !verify(&normalised, &row.code_hash).unwrap_or(false) {
            continue;
        }

        if now < row.valid_after {
            return Ok(MaturedRecovery::TooEarly);
        }

        let consumed = sqlx::query!(
            "UPDATE recovery_requests SET consumed_at = $1 WHERE id = $2 AND consumed_at IS NULL",
            now,
            row.id,
        )
        .execute(pool)
        .await?
        .rows_affected();

        return Ok(if consumed == 1 {
            MaturedRecovery::Ready {
                account_id: AccountId::from_uuid(row.account_id),
            }
        } else {
            MaturedRecovery::Unavailable
        });
    }

    Ok(MaturedRecovery::Unavailable)
}

pub fn restriction_expiry() -> DateTime<Utc> {
    crate::domain::accounts::restriction_expiry(Utc::now())
}

fn random_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

    let bytes: Vec<u8> = crate::utils::random::random_bytes(10);

    let raw: String = bytes
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect();

    format!("{}-{}", &raw[..5], &raw[5..])
}

fn normalise(code: &str) -> String {
    crate::domain::accounts::normalise_recovery_code(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::accounts::{RECOVERY_CODE_COUNT, RECOVERY_DELAY_HOURS};
    use crate::infrastructure::postgres::repositories::accounts::core;
    use chrono::Duration;

    async fn db() -> sqlx::PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    async fn account(pool: &sqlx::PgPool, name: &str) -> AccountId {
        core::create(pool, name, STATUS_ACTIVE).await.unwrap()
    }

    #[tokio::test]
    async fn test_codes_are_issued_hashed_and_unique() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;

        let codes = issue_codes(&pool, &acct).await.unwrap();
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
        let codes = issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(redeem(&pool, &codes[0]).await.unwrap(), Some(acct));
        assert_eq!(
            redeem(&pool, &codes[0]).await.unwrap(),
            None,
            "a used code must not work twice"
        );
        assert_eq!(remaining(&pool, &acct).await.unwrap(), 9);
    }

    #[tokio::test]
    async fn test_a_wrong_code_is_rejected() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(redeem(&pool, "ZZZZZ-ZZZZZ").await.unwrap(), None);
        assert_eq!(remaining(&pool, &acct).await.unwrap(), 10);
    }

    /// Reissuing must invalidate the previous set, or a printout someone threw
    /// away stays live.
    #[tokio::test]
    async fn test_reissuing_invalidates_the_old_set() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let first = issue_codes(&pool, &acct).await.unwrap();
        let second = issue_codes(&pool, &acct).await.unwrap();

        assert_eq!(redeem(&pool, &first[0]).await.unwrap(), None);
        assert_eq!(redeem(&pool, &second[0]).await.unwrap(), Some(acct));
    }

    /// One account's codes must never open another's.
    #[tokio::test]
    async fn test_a_code_only_opens_its_own_account() {
        let pool = db().await;
        let mine = account(&pool, "mine").await;
        let theirs = account(&pool, "theirs").await;

        let my_codes = issue_codes(&pool, &mine).await.unwrap();
        issue_codes(&pool, &theirs).await.unwrap();

        assert_eq!(redeem(&pool, &my_codes[0]).await.unwrap(), Some(mine));
    }

    /// The delay is the whole defence when the Matrix account is what was
    /// compromised. A code that worked immediately would make a linked Matrix
    /// account a straight backdoor.
    #[tokio::test]
    async fn test_matrix_recovery_does_nothing_for_a_day() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;

        let (code, valid_after) = request(&pool, &acct).await.unwrap();
        assert!(valid_after > Utc::now() + Duration::hours(RECOVERY_DELAY_HOURS - 1));

        assert_eq!(
            redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::TooEarly
        );

        // The countdown is visible, so the UI can say what is happening.
        assert!(pending(&pool, &acct).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn test_a_matured_request_opens_the_account() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = request(&pool, &acct).await.unwrap();

        // Wind the clock forward.
        let acct_id = acct;
        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
            Utc::now() - Duration::minutes(1),
            acct_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::Ready { account_id: acct }
        );

        // And is single-use.
        assert_eq!(
            redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::Unavailable
        );
    }

    /// The real owner, still signed in, sees the notification and stops it.
    /// This is what the 24-hour delay exists to make possible.
    #[tokio::test]
    async fn test_a_live_session_can_cancel_a_recovery() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = request(&pool, &acct).await.unwrap();

        assert_eq!(cancel_all(&pool, &acct).await.unwrap(), 1);
        assert!(pending(&pool, &acct).await.unwrap().is_none());

        // Even once it would have matured, it is dead.
        let acct_id = acct;
        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
            Utc::now() - Duration::minutes(1),
            acct_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::Unavailable
        );
    }

    /// Repeated `!recover` must not let an attacker bury the warning under a
    /// pile of requests, each with its own live code.
    #[tokio::test]
    async fn test_only_one_recovery_request_is_live_at_a_time() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;

        let (first, _) = request(&pool, &acct).await.unwrap();
        let (second, _) = request(&pool, &acct).await.unwrap();

        let acct_id = acct;
        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1 WHERE account_id = $2",
            Utc::now() - Duration::minutes(1),
            acct_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            redeem_request(&pool, &first).await.unwrap(),
            MaturedRecovery::Unavailable,
            "the superseded request must be dead"
        );
        assert_eq!(
            redeem_request(&pool, &second).await.unwrap(),
            MaturedRecovery::Ready { account_id: acct }
        );
    }

    /// An expired request cannot be redeemed even though it matured.
    #[tokio::test]
    async fn test_a_matured_request_still_expires() {
        let pool = db().await;
        let acct = account(&pool, "evan").await;
        let (code, _) = request(&pool, &acct).await.unwrap();

        let acct_id = acct;
        sqlx::query!(
            "UPDATE recovery_requests SET valid_after = $1, expires_at = $2 WHERE account_id = $3",
            Utc::now() - Duration::hours(2),
            Utc::now() - Duration::hours(1),
            acct_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            redeem_request(&pool, &code).await.unwrap(),
            MaturedRecovery::Unavailable
        );
    }
}
