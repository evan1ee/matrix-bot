//! Account recovery constants and outcomes.

use chrono::{DateTime, Duration, Utc};

use super::AccountId;

/// How many codes are issued at once.
pub const RECOVERY_CODE_COUNT: usize = 10;

/// How long a Matrix-initiated recovery waits before it can be used.
pub const RECOVERY_DELAY_HOURS: i64 = 24;

/// How long the matured code stays usable.
pub const RECOVERY_WINDOW_HOURS: i64 = 48;

/// How long a session created by recovery stays restricted.
pub const RECOVERY_RESTRICTION_HOURS: i64 = 72;

#[derive(Debug, PartialEq, Eq)]
pub enum RecoveryOutcome {
    Accepted,
    Rejected,
}

#[derive(Debug, PartialEq, Eq)]
pub enum MaturedRecovery {
    Ready { account_id: AccountId },
    TooEarly,
    Unavailable,
}

/// When a session opened by recovery stops being restricted.
pub fn restriction_expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::hours(RECOVERY_RESTRICTION_HOURS)
}
