//! When the Matrix sync loop last completed a sync.
//!
//! Written by the sync loop after every successful response, read by
//! `/health/ready` to decide whether the bot is actually talking to its
//! homeserver. An `AtomicI64` of unix millis rather than a
//! `RwLock<Option<DateTime<Utc>>>` because this is written on every sync
//! iteration (as often as every few seconds) and read on every readiness
//! probe — a lock is unnecessary contention for a single scalar, and cloning
//! `SyncHealth` must be cheap since it is carried in `BotHandles` and
//! `ApiState` alongside everything else shared between the two front doors.

use chrono::{DateTime, Duration, Utc};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

/// Sentinel meaning "never synced". `0` is not a valid unix-millis timestamp
/// for this process (that would be 1970), so it is safe to reserve.
const NEVER: i64 = 0;

/// Sentinel meaning the Matrix adapter is disabled for this deployment, so
/// `/health/ready` treats Matrix sync as vacuous (`ok: true`, `last_sync_at: None`).
const DISABLED: i64 = -1;

/// When the Matrix sync loop last completed a sync. Written by the sync loop,
/// read by `/health/ready`. Cheap to clone; lock-free.
#[derive(Clone, Default, Debug)]
pub struct SyncHealth(Arc<AtomicI64>);

impl SyncHealth {
    /// Marks Matrix sync as disabled for this deployment.
    pub fn mark_disabled(&self) {
        self.0.store(DISABLED, Ordering::Relaxed);
    }

    /// Records that a sync completed at `at`.
    pub fn record_sync(&self, at: DateTime<Utc>) {
        self.0.store(at.timestamp_millis(), Ordering::Relaxed);
    }

    /// The last recorded sync time, or `None` if a sync has never completed
    /// (or Matrix is disabled).
    pub fn last_sync(&self) -> Option<DateTime<Utc>> {
        let millis = self.0.load(Ordering::Relaxed);
        if millis <= NEVER {
            return None;
        }
        DateTime::from_timestamp_millis(millis)
    }

    /// Pure: whether the last sync is within `max_age` of `now`, or `true` when
    /// Matrix is disabled.
    ///
    /// `false` when Matrix is enabled and a sync has never completed.
    pub fn is_fresh(&self, now: DateTime<Utc>, max_age: Duration) -> bool {
        if self.0.load(Ordering::Relaxed) == DISABLED {
            return true;
        }
        match self.last_sync() {
            Some(last) => now.signed_duration_since(last) <= max_age,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_synced_has_no_last_sync_and_is_not_fresh() {
        let health = SyncHealth::default();

        assert_eq!(health.last_sync(), None);
        assert!(!health.is_fresh(Utc::now(), Duration::seconds(120)));
    }

    #[test]
    fn disabled_has_no_last_sync_and_is_always_fresh() {
        let health = SyncHealth::default();
        health.mark_disabled();

        assert_eq!(health.last_sync(), None);
        assert!(health.is_fresh(Utc::now(), Duration::seconds(120)));
    }

    #[test]
    fn record_sync_is_read_back_at_millisecond_precision() {
        let health = SyncHealth::default();
        // Millisecond precision: `DateTime::timestamp_millis` truncates any
        // finer resolution, so a `Utc::now()` with sub-millisecond precision
        // would fail a naive equality check.
        let at = Utc::now();
        let truncated = DateTime::from_timestamp_millis(at.timestamp_millis()).unwrap();

        health.record_sync(at);

        assert_eq!(health.last_sync(), Some(truncated));
    }

    #[test]
    fn is_fresh_at_the_boundary_of_max_age() {
        let health = SyncHealth::default();
        let at = Utc::now();
        health.record_sync(at);

        assert!(health.is_fresh(at + Duration::seconds(119), Duration::seconds(120)));
        assert!(!health.is_fresh(at + Duration::seconds(121), Duration::seconds(120)));
    }

    #[test]
    fn clones_share_the_same_underlying_state() {
        let health = SyncHealth::default();
        let clone = health.clone();

        let at = Utc::now();
        clone.record_sync(at);

        assert_eq!(
            health.last_sync().map(|t| t.timestamp_millis()),
            Some(at.timestamp_millis())
        );
    }
}
