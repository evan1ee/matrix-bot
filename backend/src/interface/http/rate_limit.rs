//! Thread-safe in-memory rate limiting for API endpoints.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

type RateLimiterMap = HashMap<String, (Instant, u32)>;

/// Simple sliding-window rate limiter per key.
#[derive(Clone)]
pub struct InMemoryRateLimiter {
    map: Arc<Mutex<RateLimiterMap>>,
    max_requests: u32,
    window: Duration,
}

impl InMemoryRateLimiter {
    pub fn new(max_requests: u32, window: Duration) -> Self {
        Self {
            map: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window,
        }
    }

    /// Checks if a request for `key` is permitted under the rate limit.
    /// Returns `true` if allowed, `false` if rate limit is exceeded.
    pub fn check(&self, key: &str) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        // Periodically prune stale entries to prevent unbounded memory growth
        if map.len() > 1000 {
            let prune_window = self.window * 2;
            map.retain(|_, (last_seen, _)| now.duration_since(*last_seen) < prune_window);
        }

        let entry = map.entry(key.to_string()).or_insert((now, 0));
        if now.duration_since(entry.0) >= self.window {
            *entry = (now, 1);
            true
        } else if entry.1 < self.max_requests {
            entry.1 += 1;
            true
        } else {
            false
        }
    }
}

static WEBHOOK_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();
static RECOVERY_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();
static AUTH_START_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();
static LINK_SIGNUP_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();
static TELEGRAM_WEBHOOK_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();
static GEOCODE_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();
static CALENDAR_LIMITER: OnceLock<InMemoryRateLimiter> = OnceLock::new();

/// Webhooks: max 60 requests per minute per webhook token.
pub fn check_webhook_limit(token_hash: &str) -> bool {
    WEBHOOK_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(60, Duration::from_secs(60)))
        .check(token_hash)
}

/// Recovery redemption: max 5 attempts per 15 minutes per IP.
pub fn check_recovery_limit(ip: &str) -> bool {
    RECOVERY_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(5, Duration::from_secs(900)))
        .check(ip)
}

/// Passkey registration / login ceremony start: max 30 per minute per IP.
pub fn check_auth_start_limit(ip: &str) -> bool {
    AUTH_START_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(30, Duration::from_secs(60)))
        .check(ip)
}

/// Link-token signup (unauthenticated, creates accounts): max 20 per minute per IP.
pub fn check_link_signup_limit(ip: &str) -> bool {
    LINK_SIGNUP_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(20, Duration::from_secs(60)))
        .check(ip)
}

/// Telegram webhook delivery: max 120 requests per minute per IP.
pub fn check_telegram_webhook_limit(ip: &str) -> bool {
    TELEGRAM_WEBHOOK_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(120, Duration::from_secs(60)))
        .check(ip)
}

/// Geocoding proxy: max 2 requests per 2 seconds per IP (respects Nominatim 1 req/s policy).
pub fn check_geocode_limit(ip: &str) -> bool {
    GEOCODE_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(2, Duration::from_secs(2)))
        .check(ip)
}

/// Calendar feed polling (unauthenticated): max 60 requests per minute per IP.
pub fn check_calendar_limit(ip: &str) -> bool {
    CALENDAR_LIMITER
        .get_or_init(|| InMemoryRateLimiter::new(60, Duration::from_secs(60)))
        .check(ip)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_permits_within_limit() {
        let limiter = InMemoryRateLimiter::new(3, Duration::from_secs(10));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1")); // 4th request rejected
        assert!(limiter.check("user2")); // different key is allowed
    }

    #[test]
    fn test_rate_limiter_resets_after_window() {
        let limiter = InMemoryRateLimiter::new(1, Duration::from_millis(50));
        assert!(limiter.check("user1"));
        assert!(!limiter.check("user1"));
        std::thread::sleep(Duration::from_millis(60));
        assert!(limiter.check("user1"));
    }

    #[test]
    fn calendar_limit_allows_60_then_refuses() {
        let ip = "calendar-test-ip";
        assert!((0..60).all(|_| check_calendar_limit(ip)));
        assert!(!check_calendar_limit(ip));
    }
}
