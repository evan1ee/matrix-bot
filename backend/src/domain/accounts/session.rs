//! Server-side sessions, carried in an HttpOnly cookie, and stored passkeys.
//!
//! Sessions replace a 7-day JWT held in `localStorage`. Two things changed that
//! the JWT could not offer:
//!
//! * The credential is no longer readable by JavaScript, so an XSS can ride a
//!   session while the page is open but cannot exfiltrate one that outlives it.
//! * Sessions are revocable. A JWT is valid until it expires no matter what the
//!   server thinks, which meant "sign out other devices" needed a denylist --
//!   i.e. exactly this table, without any of its other uses.
//!
//! The cookie value itself is never stored. Only its SHA-256 ([`hash_token`])
//! lands in the database, so a leaked backup cannot be replayed as a live
//! session.
//!
//! The values here are what browsers and OpenAPI already depend on: changing
//! `COOKIE_NAME` signs every user out and changing `STEP_UP_WINDOW_MINUTES`
//! changes how long a passkey assertion unlocks sensitive actions.

use super::AccountId;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id_hash: String,
    pub account_id: AccountId,
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
    /// A restricted session may register a passkey and nothing else -- see
    /// `VerifiedUser`. The window is a point in time rather than a flag so it
    /// lapses on its own, with no sweep job to forget to run.
    pub fn is_restricted(&self, now: DateTime<Utc>) -> bool {
        self.restricted_until.is_some_and(|until| now < until)
    }

    /// Whether this session may still be used at all.
    ///
    /// Both timeouts are enforced here, on read, rather than by deleting rows
    /// on a schedule, so an expiry is never waiting on a background task to
    /// notice.
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

/// A registered passkey row as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredCredential {
    pub id: String,
    pub account_id: AccountId,
    pub passkey_json: String,
    pub label: String,
    pub created_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Removed,
    NotFound,
    WouldLockOut,
}
