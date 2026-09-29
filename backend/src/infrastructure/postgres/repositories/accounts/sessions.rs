//! Sessions repository: cookie sessions, verification timestamps, and revocation.

use chrono::{DateTime, Utc};

use crate::domain::accounts::{hash_token, AccountId, Session};

struct SessionRow {
    id_hash: String,
    account_id: uuid::Uuid,
    created_at: DateTime<Utc>,
    last_seen_at: DateTime<Utc>,
    last_verified_at: DateTime<Utc>,
    restricted_until: Option<DateTime<Utc>>,
    user_agent: Option<String>,
    ip: Option<String>,
    revoked_at: Option<DateTime<Utc>>,
}

impl From<SessionRow> for Session {
    fn from(row: SessionRow) -> Self {
        Session {
            id_hash: row.id_hash,
            account_id: AccountId::from_uuid(row.account_id),
            created_at: row.created_at,
            last_seen_at: row.last_seen_at,
            last_verified_at: row.last_verified_at,
            restricted_until: row.restricted_until,
            user_agent: row.user_agent,
            ip: row.ip,
            revoked_at: row.revoked_at,
        }
    }
}

/// Opens a session and returns the cookie value, which is the only time it
/// exists anywhere outside the browser: only its SHA-256 is stored.
pub async fn create(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    user_agent: Option<&str>,
    ip: Option<&str>,
    restricted_until: Option<DateTime<Utc>>,
) -> Result<String, sqlx::Error> {
    let token = crate::utils::random::random_hex(32);
    let now = Utc::now();
    let hashed = hash_token(&token);

    sqlx::query!(
        "INSERT INTO sessions \
         (id_hash, account_id, created_at, last_seen_at, last_verified_at, \
          restricted_until, user_agent, ip) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        hashed,
        account_id.as_uuid(),
        now,
        now,
        now,
        restricted_until,
        user_agent,
        ip,
    )
    .execute(conn)
    .await?;

    Ok(token)
}

/// Resolves a cookie to a live session, refreshing its idle clock.
pub async fn resolve(pool: &sqlx::PgPool, token: &str) -> Result<Option<Session>, sqlx::Error> {
    let id_hash = hash_token(token);

    let row = sqlx::query_as!(
        SessionRow,
        "SELECT id_hash, account_id, created_at, last_seen_at, last_verified_at, \
                restricted_until, user_agent, ip, revoked_at \
         FROM sessions WHERE id_hash = $1",
        id_hash,
    )
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(None);
    };

    let session: Session = row.into();
    let now = Utc::now();
    if !session.is_live(now) {
        return Ok(None);
    }

    sqlx::query!(
        "UPDATE sessions SET last_seen_at = $1 WHERE id_hash = $2",
        now,
        id_hash,
    )
    .execute(pool)
    .await?;

    Ok(Some(Session {
        last_seen_at: now,
        ..session
    }))
}

/// Records that the holder just proved possession of a passkey.
pub async fn mark_verified(
    conn: impl sqlx::PgExecutor<'_>,
    id_hash: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE sessions SET last_verified_at = $1 WHERE id_hash = $2",
        Utc::now(),
        id_hash,
    )
    .execute(conn)
    .await?;
    Ok(())
}

pub async fn revoke(conn: impl sqlx::PgExecutor<'_>, id_hash: &str) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE sessions SET revoked_at = $1 WHERE id_hash = $2 AND revoked_at IS NULL",
        Utc::now(),
        id_hash,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Signs out every session for an account except, optionally, the one
/// asking. Used by "sign out other devices" and by account recovery.
pub async fn revoke_all_for_account(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    except: Option<&str>,
) -> Result<u64, sqlx::Error> {
    let affected = sqlx::query!(
        "UPDATE sessions SET revoked_at = $1 \
         WHERE account_id = $2 AND revoked_at IS NULL AND id_hash IS DISTINCT FROM $3",
        Utc::now(),
        account_id.as_uuid(),
        except,
    )
    .execute(conn)
    .await?
    .rows_affected();

    Ok(affected)
}

/// Live sessions for an account, newest first -- the "where you are signed
/// in" list.
pub async fn list_for_account(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Vec<Session>, sqlx::Error> {
    let rows = sqlx::query_as!(
        SessionRow,
        "SELECT id_hash, account_id, created_at, last_seen_at, last_verified_at, \
                restricted_until, user_agent, ip, revoked_at \
         FROM sessions WHERE account_id = $1 AND revoked_at IS NULL \
         ORDER BY last_seen_at DESC",
        account_id.as_uuid(),
    )
    .fetch_all(conn)
    .await?;

    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(Session::from)
        .filter(|s| s.is_live(now))
        .collect())
}

pub async fn update_last_verified_at(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    verified_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE sessions SET last_verified_at = $1 WHERE account_id = $2",
        verified_at,
        account_id.as_uuid(),
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{ABSOLUTE_TIMEOUT_DAYS, IDLE_TIMEOUT_DAYS};
    use chrono::Duration;

    async fn db() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_a", "acct_b", "acct_evan"]).await;
        pool
    }

    async fn open(pool: &crate::db::DbPool, account: &AccountId) -> String {
        create(pool, account, None, None, None).await.unwrap()
    }

    /// The cookie value must not be recoverable from the database. This is the
    /// whole reason the column is called `id_hash`.
    #[tokio::test]
    async fn test_the_raw_token_is_never_stored() {
        let pool = db().await;
        let token = open(&pool, &AccountId::from("acct_a")).await;

        let stored: Vec<String> = sqlx::query_scalar!("SELECT id_hash FROM sessions")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert_eq!(stored.len(), 1);
        assert_ne!(
            stored[0], token,
            "the cookie value itself must not be in the table"
        );
        assert_eq!(stored[0], hash_token(&token));
        assert_eq!(stored[0].len(), 64);
    }

    #[tokio::test]
    async fn test_tokens_are_unique_per_session() {
        let pool = db().await;
        let a = open(&pool, &AccountId::from("acct_a")).await;
        let b = open(&pool, &AccountId::from("acct_a")).await;
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[tokio::test]
    async fn test_resolve_returns_the_owning_account() {
        let pool = db().await;
        let evan = AccountId::from("acct_evan");
        let token = open(&pool, &evan).await;

        let session = resolve(&pool, &token).await.unwrap().unwrap();
        assert_eq!(session.account_id, evan);
        assert!(session.is_recently_verified(Utc::now()));
        assert!(!session.is_restricted(Utc::now()));
    }

    #[tokio::test]
    async fn test_an_unknown_token_resolves_to_nothing() {
        let pool = db().await;
        assert!(resolve(&pool, "nonsense").await.unwrap().is_none());
    }

    /// Revocation must take effect on the very next request, which is the one
    /// thing a stateless JWT could not do.
    #[tokio::test]
    async fn test_a_revoked_session_stops_working_immediately() {
        let pool = db().await;
        let token = open(&pool, &AccountId::from("acct_a")).await;

        assert!(resolve(&pool, &token).await.unwrap().is_some());

        revoke(&pool, &hash_token(&token)).await.unwrap();

        assert!(
            resolve(&pool, &token).await.unwrap().is_none(),
            "a revoked session must not resolve"
        );
    }

    /// Signing out other devices must not sign out the device asking.
    #[tokio::test]
    async fn test_revoking_others_spares_the_current_session() {
        let pool = db().await;
        let a = AccountId::from("acct_a");
        let mine = open(&pool, &a).await;
        let other = open(&pool, &a).await;
        let stranger = open(&pool, &AccountId::from("acct_b")).await;

        let revoked = revoke_all_for_account(&pool, &a, Some(&hash_token(&mine)))
            .await
            .unwrap();
        assert_eq!(revoked, 1);

        assert!(resolve(&pool, &mine).await.unwrap().is_some());
        assert!(resolve(&pool, &other).await.unwrap().is_none());
        assert!(
            resolve(&pool, &stranger).await.unwrap().is_some(),
            "another account's sessions must be untouched"
        );
    }

    #[tokio::test]
    async fn test_listing_shows_only_this_accounts_live_sessions() {
        let pool = db().await;
        let a = AccountId::from("acct_a");
        let _mine = open(&pool, &a).await;
        let gone = open(&pool, &a).await;
        let _theirs = open(&pool, &AccountId::from("acct_b")).await;

        revoke(&pool, &hash_token(&gone)).await.unwrap();

        let listed = list_for_account(&pool, &a).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].account_id, a);
    }

    fn fresh(now: DateTime<Utc>) -> Session {
        Session {
            id_hash: "h".into(),
            account_id: AccountId::from("acct_a"),
            created_at: now,
            last_seen_at: now,
            last_verified_at: now,
            restricted_until: None,
            user_agent: None,
            ip: None,
            revoked_at: None,
        }
    }

    /// Both timeouts are checked on read, so neither depends on a sweep job.
    #[test]
    fn test_timeouts_are_enforced_without_a_background_job() {
        let now = Utc::now();
        let fresh = fresh(now);
        assert!(fresh.is_live(now));

        let idle = Session {
            last_seen_at: now - Duration::days(IDLE_TIMEOUT_DAYS + 1),
            ..fresh.clone()
        };
        assert!(!idle.is_live(now), "an idle session must expire");

        let ancient = Session {
            created_at: now - Duration::days(ABSOLUTE_TIMEOUT_DAYS + 1),
            ..fresh.clone()
        };
        assert!(
            !ancient.is_live(now),
            "a session must not outlive the absolute cap"
        );
    }

    /// A restriction lapses on its own, with nothing to run to clear it.
    #[test]
    fn test_restriction_expires_on_its_own() {
        let now = Utc::now();
        let restricted = Session {
            restricted_until: Some(now + Duration::hours(72)),
            ..fresh(now)
        };
        assert!(restricted.is_restricted(now));
        assert!(!restricted.is_restricted(now + Duration::hours(73)));
    }
}
