//! The SSE hub: this instance's connected browsers, keyed by account.
//!
//! Ownership is structural — a subscription is registered only under the
//! account that authenticated it, and a line is only ever pushed to that
//! account's subscriptions. A subscriber whose bounded channel is full is
//! dropped (never awaited): its browser reconnects and refetches.

use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use futures_util::Stream;
use tokio::sync::mpsc;
use tracing::warn;

use crate::application::outbox::{OutboxService, StoredEvent};
use crate::domain::accounts::AccountId;
use crate::domain::events::EventId;

#[derive(Debug, PartialEq, Eq)]
pub enum Notified {
    NoSubscriber,
    Missing,
    Delivered(usize),
    LoadFailed,
}

/// One registered subscription: its key and the sending half of its channel.
type Registered = (u64, mpsc::Sender<Arc<str>>);

#[derive(Default)]
struct State {
    next_key: u64,
    subscribers: HashMap<AccountId, Vec<Registered>>,
    /// Highest event id pushed; replay starts just below it.
    last_seen: i64,
}

#[derive(Clone)]
pub struct Hub {
    state: Arc<Mutex<State>>,
    outbox: OutboxService,
    capacity: usize,
}

pub struct Subscription {
    rx: mpsc::Receiver<Arc<str>>,
    _guard: Guard,
}

struct Guard {
    state: Arc<Mutex<State>>,
    account: AccountId,
    key: u64,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if let Some(list) = state.subscribers.get_mut(&self.account) {
            list.retain(|(k, _)| *k != self.key);
            if list.is_empty() {
                state.subscribers.remove(&self.account);
            }
        }
    }
}

impl Stream for Subscription {
    type Item = Arc<str>;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.rx.poll_recv(cx)
    }
}

impl Hub {
    pub fn new(outbox: OutboxService, capacity: usize) -> Self {
        Self {
            state: Arc::default(),
            outbox,
            capacity,
        }
    }

    pub fn subscribe(&self, account: AccountId) -> Subscription {
        let (tx, rx) = mpsc::channel(self.capacity);
        let key = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            state.next_key += 1;
            let key = state.next_key;
            state
                .subscribers
                .entry(account)
                .or_default()
                .push((key, tx));
            key
        };
        Subscription {
            rx,
            _guard: Guard {
                state: self.state.clone(),
                account,
                key,
            },
        }
    }

    fn has_subscriber(&self, account: &AccountId) -> bool {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.subscribers.contains_key(account)
    }

    fn accounts(&self) -> Vec<AccountId> {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.subscribers.keys().copied().collect()
    }

    /// Pushes one line to every subscription of `account`; returns how many
    /// received it. Never blocks: a full or closed channel is removed.
    pub(crate) fn push(&self, account: &AccountId, line: Arc<str>) -> usize {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        let Some(list) = state.subscribers.get_mut(account) else {
            return 0;
        };
        list.retain(|(_, tx)| tx.try_send(line.clone()).is_ok());
        let n = list.len();
        if n == 0 {
            state.subscribers.remove(account);
        }
        n
    }

    fn forward(&self, event: &StoredEvent) -> usize {
        let line: Arc<str> = Arc::from(event.json.to_string());
        let n = self.push(&event.account_id, line);
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.last_seen = state.last_seen.max(event.id.0);
        n
    }

    pub(crate) async fn on_notification(&self, id: EventId, account: AccountId) -> Notified {
        if !self.has_subscriber(&account) {
            return Notified::NoSubscriber;
        }
        match self.outbox.load(id).await {
            Ok(Some(event)) => Notified::Delivered(self.forward(&event)),
            Ok(None) => Notified::Missing,
            Err(err) => {
                warn!(error = %err, event_id = id.0, "Could not load an outbox event for SSE");
                Notified::LoadFailed
            }
        }
    }

    /// After a listener reconnect: re-sends recent events for connected
    /// accounts. `window` reaches below `last_seen` because identity ids can
    /// commit out of order; duplicates only re-invalidate browser caches.
    pub(crate) async fn replay(&self, window: i64, limit: i64) {
        let accounts = self.accounts();
        if accounts.is_empty() {
            return;
        }
        let after = {
            let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            EventId((state.last_seen - window).max(0))
        };
        match self.outbox.load_since(&accounts, after, limit).await {
            Ok(events) => {
                for event in &events {
                    self.forward(event);
                }
            }
            Err(err) => warn!(error = %err, "SSE replay after a listener reconnect failed"),
        }
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures_util::StreamExt;

    use super::*;
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::domain::events::SystemEvent;

    async fn services() -> AppServices {
        let pool = crate::db::pool::test_pool_sized(3).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_mine", "acct_theirs"]).await;
        AppServices::new(&pool)
    }

    /// Given subscribers for two accounts, When a line is pushed for one,
    /// Then only that account's subscriber receives it.
    #[tokio::test]
    async fn a_subscriber_only_sees_its_own_account() {
        let hub = Hub::new(services().await.outbox, 8);
        let mine = AccountId::from_seed("acct_mine");
        let theirs = AccountId::from_seed("acct_theirs");
        let mut my_sub = hub.subscribe(mine);
        let mut their_sub = hub.subscribe(theirs);

        assert_eq!(hub.push(&theirs, Arc::from("theirs")), 1);
        assert_eq!(hub.push(&mine, Arc::from("mine")), 1);

        assert_eq!(&*my_sub.next().await.unwrap(), "mine");
        assert_eq!(&*their_sub.next().await.unwrap(), "theirs");
        assert!(
            tokio::time::timeout(Duration::from_millis(50), my_sub.next())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_notification_for_an_account_with_no_subscriber_is_not_loaded() {
        let svc = services().await;
        let hub = Hub::new(svc.outbox.clone(), 8);
        // No row with this id exists; a load would return Missing.
        let n = hub
            .on_notification(EventId(i64::MAX), AccountId::from_seed("acct_mine"))
            .await;
        assert_eq!(n, Notified::NoSubscriber);
    }

    #[tokio::test]
    async fn a_notification_loads_and_forwards_the_stored_json() {
        let svc = services().await;
        let owner = AccountId::from_seed("acct_mine");
        let mut tx = svc.outbox.pool().begin().await.unwrap();
        let published = crate::infrastructure::postgres::repositories::outbox::publish(
            &mut tx,
            &SystemEvent::AiConfigUpdated { owner_id: owner },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let hub = Hub::new(svc.outbox.clone(), 8);
        let mut sub = hub.subscribe(owner);
        assert_eq!(
            hub.on_notification(published.event_id, owner).await,
            Notified::Delivered(1)
        );
        let line: serde_json::Value = serde_json::from_str(&sub.next().await.unwrap()).unwrap();
        assert_eq!(line["type"], "AiConfigUpdated");
    }

    /// Given events committed while the listener was down (no NOTIFY seen),
    /// When the hub replays, Then connected subscribers receive them and
    /// other accounts' events stay out.
    #[tokio::test]
    async fn replay_sends_missed_events_to_connected_accounts_only() {
        let svc = services().await;
        let mine = AccountId::from_seed("acct_mine");
        let theirs = AccountId::from_seed("acct_theirs");
        let mut tx = svc.outbox.pool().begin().await.unwrap();
        for owner in [mine, theirs] {
            crate::infrastructure::postgres::repositories::outbox::publish(
                &mut tx,
                &SystemEvent::AiConfigUpdated { owner_id: owner },
            )
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let hub = Hub::new(svc.outbox.clone(), 8);
        let mut sub = hub.subscribe(mine);
        hub.replay(100, 500).await;

        let line: serde_json::Value = serde_json::from_str(&sub.next().await.unwrap()).unwrap();
        assert_eq!(line["payload"]["owner_id"], mine.to_string());
        assert!(tokio::time::timeout(Duration::from_millis(50), sub.next())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn a_full_subscriber_is_dropped_without_blocking_others() {
        let hub = Hub::new(services().await.outbox, 1);
        let owner = AccountId::from_seed("acct_mine");
        let slow = hub.subscribe(owner);
        let mut fast = hub.subscribe(owner);
        assert_eq!(hub.push(&owner, Arc::from("1")), 2);
        let _ = fast.next().await;
        // `slow` never reads: its one-slot channel is full, so it is dropped.
        assert_eq!(hub.push(&owner, Arc::from("2")), 1);
        drop(slow);
        assert_eq!(&*fast.next().await.unwrap(), "2");
    }

    #[tokio::test]
    async fn dropping_a_subscription_unregisters_it() {
        let hub = Hub::new(services().await.outbox, 8);
        let owner = AccountId::from_seed("acct_mine");
        drop(hub.subscribe(owner));
        assert_eq!(hub.push(&owner, Arc::from("x")), 0);
    }
}
