//! A minimal rate gate shared by the adapters' progress sinks (Discord edits and
//! typing pulses, Telegram typing and status edits).

use std::time::{Duration, Instant};

/// At most one action per `gap`.
pub struct Throttle {
    gap: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(gap: Duration) -> Self {
        Self { gap, last: None }
    }

    pub fn ready(&mut self, now: Instant) -> bool {
        match self.last {
            Some(last) if now.saturating_duration_since(last) < self.gap => false,
            _ => {
                self.last = Some(now);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_throttle_allows_one_action_per_gap() {
        let start = Instant::now();
        let mut t = Throttle::new(Duration::from_secs(1));
        assert!(t.ready(start));
        assert!(!t.ready(start + Duration::from_millis(500)));
        assert!(t.ready(start + Duration::from_millis(1000)));
        assert!(!t.ready(start + Duration::from_millis(1999)));
        assert!(t.ready(start + Duration::from_secs(3)));
    }
}
