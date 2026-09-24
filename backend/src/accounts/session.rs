//! Server-side sessions, carried in an HttpOnly cookie.
//!
//! This replaces a 7-day JWT held in `localStorage`. Two things changed that
//! the JWT could not offer:
//!
//! * The credential is no longer readable by JavaScript, so an XSS can ride a
//!   session while the page is open but cannot exfiltrate one that outlives it.
//! * Sessions are revocable. A JWT is valid until it expires no matter what the
//!   server thinks, which meant "sign out other devices" needed a denylist —
//!   i.e. exactly this table, without any of its other uses.
//!
//! The cookie value itself is never stored. Only its SHA-256 lands in the
//! database, so a leaked backup cannot be replayed as a live session, the same
//! reasoning `api/auth/types.rs` applies to its token cache.

use crate::db::DbPool;
use chrono::{DateTime, Duration, Utc};
use sha2::{Digest, Sha256};

/// The cookie a browser presents to prove who it is.
pub const COOKIE_NAME: &str = "mb_session";

/// How long a session survives without being used.
pub const IDLE_TIMEOUT_DAYS: i64 = 30;

/// How long a session survives at all, however active.
pub const ABSOLUTE_TIMEOUT_DAYS: i64 = 90;

/// How recent a passkey assertion must be for a sensitive action.
///
/// Five minutes, matching the "sudo mode" convention users already know from
/// GitHub. Long enough to link an account without re-tapping a key mid-flow,
/// short enough that a walked-away-from laptop is not an unlocked one.
pub const STEP_UP_WINDOW_MINUTES: i64 = 5;

/// A live session, as the extractor sees it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Session {
    pub id_hash: String,
    pub account_id: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub last_verified_at: DateTime<Utc>,
    pub restricted_until: Option<DateTime<Utc>>,
    pub user_agent: Option<String>,
    pub ip: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl Session {
    /// Whether a passkey assertion happened recently enough for a sensitive
    /// action.
    pub fn is_recently_verified(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.last_verified_at) < Duration::minutes(STEP_UP_WINDOW_MINUTES)
    }

    /// Whether this session came out of account recovery and is still inside
    /// its restricted window.
    ///
    /// A restricted session may register a passkey and nothing else — see
    /// `VerifiedUser`. The window is a point in time rather than a flag so it
    /// lapses on its own, with no sweep job to forget to run.
    pub fn is_restricted(&self, now: DateTime<Utc>) -> bool {
        self.restricted_until.is_some_and(|until| now < until)
    }

    /// Whether this session may still be used at all.
    ///
    /// Both timeouts are enforced here rather than by deleting rows on a
    /// schedule, so an expiry is never waiting on a background task to notice.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none()
            && now.signed_duration_since(self.last_seen_at) < Duration::days(IDLE_TIMEOUT_DAYS)
            && now.signed_duration_since(self.created_at) < Duration::days(ABSOLUTE_TIMEOUT_DAYS)
    }
}

/// Hashes a session cookie into its storage key.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

pub struct SessionRepository;

impl SessionRepository {
    /// Opens a session and returns the cookie value, which is the only time it
    /// exists anywhere outside the browser.
    ///
    /// `restricted_until` is `Some` only for a session minted by account
    /// recovery.
    pub async fn create(
        pool: &DbPool,
        account_id: &str,
        user_agent: Option<&str>,
        ip: Option<&str>,
        restricted_until: Option<DateTime<Utc>>,
    ) -> Result<String, sqlx::Error> {
        // Minted from the OS CSPRNG for the same reason account ids are (see
        // utils/random.rs): 32 random bytes is more entropy than a session
        // token needs.
        let token = crate::utils::random::random_hex(32);

        let now = Utc::now();
        crate::db_execute!(
            pool,
            "INSERT INTO sessions \
             (id_hash, account_id, created_at, last_seen_at, last_verified_at, \
              restricted_until, user_agent, ip) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            hash_token(&token),
            account_id,
            now,
            now,
            now,
            restricted_until,
            user_agent,
            ip
        )?;

        Ok(token)
    }

    /// Resolves a cookie to a live session, refreshing its idle clock.
    ///
    /// Returns `None` for anything not currently usable — unknown, revoked, or
    /// past either timeout — so a caller cannot accidentally treat an expired
    /// session as merely stale.
    pub async fn resolve(pool: &DbPool, token: &str) -> Result<Option<Session>, sqlx::Error> {
        let id_hash = hash_token(token);

        let Some(session) = crate::db_fetch_optional!(
            pool,
            Session,
            "SELECT id_hash, account_id, created_at, last_seen_at, last_verified_at, \
                    restricted_until, user_agent, ip, revoked_at \
             FROM sessions WHERE id_hash = ?",
            &id_hash
        )?
        else {
            return Ok(None);
        };

        let now = Utc::now();
        if !session.is_live(now) {
            return Ok(None);
        }

        crate::db_execute!(
            pool,
            "UPDATE sessions SET last_seen_at = ? WHERE id_hash = ?",
            now,
            &id_hash
        )?;

        Ok(Some(Session {
            last_seen_at: now,
            ..session
        }))
    }

    /// Records that the holder just proved possession of a passkey.
    ///
    /// This is what opens the step-up window, so it is called only from a
    /// finished WebAuthn assertion — never from an ordinary request.
    pub async fn mark_verified(pool: &DbPool, id_hash: &str) -> Result<(), sqlx::Error> {
        crate::db_execute!(
            pool,
            "UPDATE sessions SET last_verified_at = ? WHERE id_hash = ?",
            Utc::now(),
            id_hash
        )?;
        Ok(())
    }

    pub async fn revoke(pool: &DbPool, id_hash: &str) -> Result<(), sqlx::Error> {
        crate::db_execute!(
            pool,
            "UPDATE sessions SET revoked_at = ? WHERE id_hash = ? AND revoked_at IS NULL",
            Utc::now(),
            id_hash
        )?;
        Ok(())
    }

    /// Signs out every session for an account except, optionally, the one
    /// asking. Used by "sign out other devices" and by account recovery.
    pub async fn revoke_all_for_account(
        pool: &DbPool,
        account_id: &str,
        except: Option<&str>,
    ) -> Result<u64, sqlx::Error> {
        let affected = crate::db_execute!(
            pool,
            "UPDATE sessions SET revoked_at = ? \
             WHERE account_id = ? AND revoked_at IS NULL AND id_hash IS DISTINCT FROM ?",
            Utc::now(),
            account_id,
            except
        )?;

        Ok(affected)
    }

    /// Live sessions for an account, newest first — the "where you're signed
    /// in" list.
    pub async fn list_for_account(
        pool: &DbPool,
        account_id: &str,
    ) -> Result<Vec<Session>, sqlx::Error> {
        let sessions = crate::db_fetch_all!(
            pool,
            Session,
            "SELECT id_hash, account_id, created_at, last_seen_at, last_verified_at, \
                    restricted_until, user_agent, ip, revoked_at \
             FROM sessions WHERE account_id = ? AND revoked_at IS NULL \
             ORDER BY last_seen_at DESC",
            account_id
        )?;

        let now = Utc::now();
        Ok(sessions.into_iter().filter(|s| s.is_live(now)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    /// The cookie value must not be recoverable from the database. This is the
    /// whole reason the column is called `id_hash`.
    #[tokio::test]
    async fn test_the_raw_token_is_never_stored() {
        let pool = db().await;
        let token = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();

        let stored: Vec<String> =
            crate::db_scalar_all!(&pool, _, "SELECT id_hash FROM sessions").unwrap();

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
        let a = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();
        let b = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
    }

    #[tokio::test]
    async fn test_resolve_returns_the_owning_account() {
        let pool = db().await;
        let token = SessionRepository::create(&pool, "acct_evan", None, None, None)
            .await
            .unwrap();

        let session = SessionRepository::resolve(&pool, &token)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.account_id, "acct_evan");
        assert!(session.is_recently_verified(Utc::now()));
        assert!(!session.is_restricted(Utc::now()));
    }

    #[tokio::test]
    async fn test_an_unknown_token_resolves_to_nothing() {
        let pool = db().await;
        assert!(SessionRepository::resolve(&pool, "nonsense")
            .await
            .unwrap()
            .is_none());
    }

    /// Revocation must take effect on the very next request, which is the one
    /// thing a stateless JWT could not do.
    #[tokio::test]
    async fn test_a_revoked_session_stops_working_immediately() {
        let pool = db().await;
        let token = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();

        assert!(SessionRepository::resolve(&pool, &token)
            .await
            .unwrap()
            .is_some());

        SessionRepository::revoke(&pool, &hash_token(&token))
            .await
            .unwrap();

        assert!(
            SessionRepository::resolve(&pool, &token)
                .await
                .unwrap()
                .is_none(),
            "a revoked session must not resolve"
        );
    }

    /// Signing out other devices must not sign out the device asking.
    #[tokio::test]
    async fn test_revoking_others_spares_the_current_session() {
        let pool = db().await;
        let mine = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();
        let other = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();
        let stranger = SessionRepository::create(&pool, "acct_b", None, None, None)
            .await
            .unwrap();

        let revoked =
            SessionRepository::revoke_all_for_account(&pool, "acct_a", Some(&hash_token(&mine)))
                .await
                .unwrap();
        assert_eq!(revoked, 1);

        assert!(SessionRepository::resolve(&pool, &mine)
            .await
            .unwrap()
            .is_some());
        assert!(SessionRepository::resolve(&pool, &other)
            .await
            .unwrap()
            .is_none());
        assert!(
            SessionRepository::resolve(&pool, &stranger)
                .await
                .unwrap()
                .is_some(),
            "another account's sessions must be untouched"
        );
    }

    #[tokio::test]
    async fn test_listing_shows_only_this_accounts_live_sessions() {
        let pool = db().await;
        let _mine = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();
        let gone = SessionRepository::create(&pool, "acct_a", None, None, None)
            .await
            .unwrap();
        let _theirs = SessionRepository::create(&pool, "acct_b", None, None, None)
            .await
            .unwrap();

        SessionRepository::revoke(&pool, &hash_token(&gone))
            .await
            .unwrap();

        let listed = SessionRepository::list_for_account(&pool, "acct_a")
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].account_id, "acct_a");
    }

    /// Both timeouts are checked on read, so neither depends on a sweep job.
    #[test]
    fn test_timeouts_are_enforced_without_a_background_job() {
        let now = Utc::now();
        let fresh = Session {
            id_hash: "h".into(),
            account_id: "acct_a".into(),
            created_at: now,
            last_seen_at: now,
            last_verified_at: now,
            restricted_until: None,
            user_agent: None,
            ip: None,
            revoked_at: None,
        };
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

    /// The step-up window is what makes "recent passkey verification" mean
    /// anything, so its boundary is asserted on both sides.
    #[test]
    fn test_step_up_window_boundary() {
        let now = Utc::now();
        let session = Session {
            id_hash: "h".into(),
            account_id: "acct_a".into(),
            created_at: now,
            last_seen_at: now,
            last_verified_at: now - Duration::minutes(STEP_UP_WINDOW_MINUTES)
                + Duration::seconds(1),
            restricted_until: None,
            user_agent: None,
            ip: None,
            revoked_at: None,
        };
        assert!(session.is_recently_verified(now), "just inside the window");

        let stale = Session {
            last_verified_at: now
                - Duration::minutes(STEP_UP_WINDOW_MINUTES)
                - Duration::seconds(1),
            ..session
        };
        assert!(!stale.is_recently_verified(now), "just outside the window");
    }

    /// A restriction lapses on its own, with nothing to run to clear it.
    #[test]
    fn test_restriction_expires_on_its_own() {
        let now = Utc::now();
        let restricted = Session {
            id_hash: "h".into(),
            account_id: "acct_a".into(),
            created_at: now,
            last_seen_at: now,
            last_verified_at: now,
            restricted_until: Some(now + Duration::hours(72)),
            user_agent: None,
            ip: None,
            revoked_at: None,
        };
        assert!(restricted.is_restricted(now));
        assert!(!restricted.is_restricted(now + Duration::hours(73)));
    }
}
