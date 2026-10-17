//! In-memory bounded cache tracking authentic AI-generated replies.
//!
//! Prevents quote-based confirmation loops (TD-15) by verifying that quoted
//! messages were authentic assistant turn responses emitted for the same
//! room and owner.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::domain::accounts::AccountId;

/// Default TTL for tracked AI replies (1 hour, matching session TTL).
pub const DEFAULT_TTL: Duration = Duration::from_secs(3600);

/// Maximum number of tracked AI reply IDs kept in memory.
pub const DEFAULT_MAX_CAPACITY: usize = 1000;

#[derive(Debug, Clone)]
struct AuthenticReplyEntry {
    room_key: String,
    event_id: String,
    owner_id: AccountId,
    inserted_at: Instant,
    seq: u64,
}

/// Tracks authentic AI replies across rooms and owners.
#[derive(Debug)]
pub struct AiReplyTracker {
    ttl: Duration,
    max_capacity: usize,
    entries: Mutex<HashMap<String, AuthenticReplyEntry>>,
    next_seq: AtomicU64,
}

impl Default for AiReplyTracker {
    fn default() -> Self {
        Self::new(DEFAULT_TTL, DEFAULT_MAX_CAPACITY)
    }
}

impl AiReplyTracker {
    /// Creates a new `AiReplyTracker` with the given TTL and capacity limit.
    pub fn new(ttl: Duration, max_capacity: usize) -> Self {
        Self {
            ttl,
            max_capacity,
            entries: Mutex::new(HashMap::new()),
            next_seq: AtomicU64::new(1),
        }
    }

    /// Records an authentic AI reply event ID for a given room and owner.
    pub fn record(&self, room_key: &str, event_id: &str, owner_id: AccountId) {
        if self.max_capacity == 0 {
            return;
        }

        let key = format!("{room_key}:{event_id}");
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());

        let now = Instant::now();
        // Purge expired entries
        map.retain(|_, entry| now.saturating_duration_since(entry.inserted_at) < self.ttl);

        // Evict oldest if at capacity
        if !map.contains_key(&key) {
            while map.len() >= self.max_capacity {
                if let Some(oldest_key) = map
                    .iter()
                    .min_by_key(|(_, entry)| entry.seq)
                    .map(|(k, _)| k.clone())
                {
                    map.remove(&oldest_key);
                } else {
                    break;
                }
            }
        }

        map.insert(
            key,
            AuthenticReplyEntry {
                room_key: room_key.to_string(),
                event_id: event_id.to_string(),
                owner_id,
                inserted_at: now,
                seq,
            },
        );
    }

    /// Checks whether the given event ID is an authentic, unexpired AI reply
    /// for the specified room and owner.
    pub fn is_authentic(&self, room_key: &str, event_id: &str, owner_id: &AccountId) -> bool {
        let key = format!("{room_key}:{event_id}");
        let map = self.entries.lock().unwrap_or_else(|p| p.into_inner());

        if let Some(entry) = map.get(&key) {
            if entry.room_key == room_key
                && entry.event_id == event_id
                && entry.owner_id == *owner_id
                && entry.inserted_at.elapsed() < self.ttl
            {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_record_and_validate_authentic_reply() {
        let tracker = AiReplyTracker::new(Duration::from_secs(60), 100);
        let owner = AccountId::generate();
        tracker.record("!room1:server", "$event1", owner);

        assert!(tracker.is_authentic("!room1:server", "$event1", &owner));
        assert!(!tracker.is_authentic("!room2:server", "$event1", &owner)); // wrong room
        assert!(!tracker.is_authentic("!room1:server", "$event2", &owner)); // wrong event
        let other_owner = AccountId::generate();
        assert!(!tracker.is_authentic("!room1:server", "$event1", &other_owner)); // wrong owner
    }

    #[test]
    fn test_expired_reply_is_not_authentic() {
        let tracker = AiReplyTracker::new(Duration::from_millis(10), 100);
        let owner = AccountId::generate();
        tracker.record("!room1:server", "$event1", owner);
        std::thread::sleep(Duration::from_millis(20));
        assert!(!tracker.is_authentic("!room1:server", "$event1", &owner));
    }

    #[test]
    fn test_capacity_bounding() {
        let tracker = AiReplyTracker::new(Duration::from_secs(60), 2);
        let owner = AccountId::generate();
        tracker.record("!room:server", "$ev1", owner);
        tracker.record("!room:server", "$ev2", owner);
        tracker.record("!room:server", "$ev3", owner);
        // $ev1 should be evicted when capacity of 2 is exceeded
        assert!(!tracker.is_authentic("!room:server", "$ev1", &owner));
        assert!(tracker.is_authentic("!room:server", "$ev2", &owner));
        assert!(tracker.is_authentic("!room:server", "$ev3", &owner));
    }

    #[test]
    fn test_default_tracker_initialization() {
        let tracker = AiReplyTracker::default();
        assert_eq!(tracker.ttl, DEFAULT_TTL);
        assert_eq!(tracker.max_capacity, DEFAULT_MAX_CAPACITY);
    }
}
