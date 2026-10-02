//! The bounded update queue every Telegram update passes through (T11), and
//! the de-duplication of redelivered updates.
//!
//! Two sources feed one queue: the webhook endpoint (`submit`, never blocking)
//! and the polling loop (`push_wait`, which waits for capacity). No teloxide type
//! appears here: the queue carries the raw update JSON.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

use axum::body::Bytes;
use tokio::sync::mpsc;

use crate::utils::secure::constant_time_eq;

/// Updates waiting for a worker; beyond this the webhook answers 503 and
/// Telegram redelivers later, so nothing is lost.
pub const QUEUE_CAPACITY: usize = 64;

/// The queue is full (or closed): the caller should answer 503.
#[derive(Debug, PartialEq, Eq)]
pub struct Full;

/// The handle the webhook endpoint (and the polling loop) feed.
#[derive(Clone)]
pub struct TelegramIngress {
    tx: mpsc::Sender<Bytes>,
    /// `None` outside webhook mode: nothing can match, so nothing is accepted.
    secret: Option<Arc<str>>,
}

impl TelegramIngress {
    pub fn channel(capacity: usize, secret: Option<&str>) -> (Self, mpsc::Receiver<Bytes>) {
        let (tx, rx) = mpsc::channel(capacity);
        let secret = secret.map(Arc::from);
        (Self { tx, secret }, rx)
    }

    /// T9: the `X-Telegram-Bot-Api-Secret-Token` header against the configured
    /// secret, in constant time. A missing header or secret never matches.
    pub fn secret_matches(&self, header: Option<&[u8]>) -> bool {
        match (&self.secret, header) {
            (Some(secret), Some(given)) => constant_time_eq(secret.as_bytes(), given),
            _ => false,
        }
    }

    /// Enqueues without waiting; a full or closed queue is `Full`.
    pub fn submit(&self, body: Bytes) -> Result<(), Full> {
        self.tx.try_send(body).map_err(|_| Full)
    }

    /// Enqueues, waiting for capacity (polling: a saturated pool slows polling
    /// down instead of dropping updates). `Full` only when the queue is closed.
    pub(super) async fn push_wait(&self, body: Bytes) -> Result<(), Full> {
        self.tx.send(body).await.map_err(|_| Full)
    }
}

/// Bounded `update_id` de-duplication (T11): remembers the last `capacity` ids.
/// Webhook redelivery and polling restarts are at-least-once.
pub struct RecentUpdates {
    capacity: usize,
    order: VecDeque<u32>,
    seen: HashSet<u32>,
}

impl RecentUpdates {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            order: VecDeque::with_capacity(capacity),
            seen: HashSet::with_capacity(capacity),
        }
    }

    /// `true` the first time `id` is seen within the window, `false` after.
    pub fn first_time(&mut self, id: u32) -> bool {
        if self.capacity == 0 {
            return true;
        }
        if !self.seen.insert(id) {
            return false;
        }
        self.order.push_back(id);
        while self.order.len() > self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.seen.remove(&old);
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seen_update_id_is_not_first_time_again() {
        let mut recent = RecentUpdates::new(3);
        assert!(recent.first_time(1));
        assert!(!recent.first_time(1));
        assert!(recent.first_time(2));
    }

    #[test]
    fn the_oldest_id_is_forgotten_after_capacity() {
        let mut recent = RecentUpdates::new(3);
        for id in 1..=3 {
            assert!(recent.first_time(id));
        }
        assert!(recent.first_time(4)); // evicts 1
        assert!(!recent.first_time(3));
        assert!(!recent.first_time(4));
        assert!(recent.first_time(1)); // forgotten, so new again
    }

    #[test]
    fn a_zero_capacity_still_remembers_nothing_without_panicking() {
        let mut recent = RecentUpdates::new(0);
        assert!(recent.first_time(1));
        assert!(recent.first_time(1));
    }

    #[test]
    fn only_the_exact_secret_matches() {
        let (ingress, _rx) = TelegramIngress::channel(2, Some("s3cret-_"));
        assert!(ingress.secret_matches(Some(b"s3cret-_")));
        assert!(!ingress.secret_matches(Some(b"s3cret-x")));
        assert!(!ingress.secret_matches(Some(b"s3cret")));
        assert!(!ingress.secret_matches(Some(b"s3cret-_-")));
        assert!(!ingress.secret_matches(None));
    }

    #[test]
    fn without_a_configured_secret_nothing_matches() {
        let (ingress, _rx) = TelegramIngress::channel(2, None);
        assert!(!ingress.secret_matches(Some(b"")));
        assert!(!ingress.secret_matches(None));
    }

    #[tokio::test]
    async fn submit_never_blocks_and_a_full_queue_is_an_error() {
        let (ingress, mut rx) = TelegramIngress::channel(2, Some("s"));
        assert!(ingress.submit(Bytes::from_static(b"1")).is_ok());
        assert!(ingress.submit(Bytes::from_static(b"2")).is_ok());
        assert_eq!(ingress.submit(Bytes::from_static(b"3")), Err(Full));
        assert_eq!(rx.recv().await.unwrap(), Bytes::from_static(b"1"));
        assert!(ingress.submit(Bytes::from_static(b"3")).is_ok());
    }

    #[test]
    fn the_default_queue_holds_sixty_four() {
        assert_eq!(QUEUE_CAPACITY, 64);
    }
}
