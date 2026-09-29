//! Sessions and passkey credentials.
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
    pub fn is_recently_verified(&self, now: DateTime<Utc>) -> bool {
        now.signed_duration_since(self.last_verified_at) < Duration::minutes(STEP_UP_WINDOW_MINUTES)
    }

    pub fn is_restricted(&self, now: DateTime<Utc>) -> bool {
        self.restricted_until.is_some_and(|until| now < until)
    }

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

impl StoredCredential {
    pub fn passkey(&self) -> Option<webauthn_rs::prelude::Passkey> {
        match serde_json::from_str(&self.passkey_json) {
            Ok(passkey) => Some(passkey),
            Err(err) => {
                tracing::error!("Credential {} will not deserialise: {:?}", self.id, err);
                None
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DeleteOutcome {
    Removed,
    NotFound,
    WouldLockOut,
}
