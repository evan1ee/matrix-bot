//! The per-account cap on concurrent streaming AI turns, and the task tracker
//! the composition root drains on shutdown.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tokio_util::task::TaskTracker;

use crate::domain::accounts::AccountId;

/// How many streaming turns one account may run at once.
pub const MAX_TURNS_PER_ACCOUNT: usize = 2;

/// Running streaming turns: a count per account and the tracker their tasks
/// are spawned on. Cheap to clone; every clone shares the same counts.
#[derive(Clone)]
pub struct AiTurns {
    inner: Arc<Inner>,
}

struct Inner {
    per_account: usize,
    // A std mutex: it is held only to bump a counter, never across `.await`.
    running: Mutex<HashMap<AccountId, usize>>,
    tracker: TaskTracker,
}

impl Inner {
    fn running(&self) -> MutexGuard<'_, HashMap<AccountId, usize>> {
        // A panic while holding this lock cannot leave the map half-updated
        // (each critical section is one insert or remove), so a poisoned
        // lock is still safe to use.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl AiTurns {
    pub fn new(per_account: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                per_account,
                running: Mutex::new(HashMap::new()),
                tracker: TaskTracker::new(),
            }),
        }
    }

    /// A slot for one more turn of `owner`'s, or `None` when the account
    /// already runs `per_account` turns.
    pub fn try_acquire(&self, owner: &AccountId) -> Option<TurnPermit> {
        let mut running = self.inner.running();
        let count = running.entry(*owner).or_insert(0);
        if *count >= self.inner.per_account {
            return None;
        }
        *count += 1;
        Some(TurnPermit {
            turns: self.inner.clone(),
            owner: *owner,
        })
    }

    /// Where turn tasks are spawned, so shutdown can wait for them.
    pub fn tracker(&self) -> &TaskTracker {
        &self.inner.tracker
    }
}

/// One running turn's slot. Released on `Drop`, so a turn that ends any way
/// at all (success, error, cancellation, panic unwinding) frees it.
pub struct TurnPermit {
    turns: Arc<Inner>,
    owner: AccountId,
}

impl Drop for TurnPermit {
    fn drop(&mut self) {
        let mut running = self.turns.running();
        if let Some(count) = running.get_mut(&self.owner) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                running.remove(&self.owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_account_gets_at_most_its_cap_and_dropping_frees_a_slot() {
        let turns = AiTurns::new(2);
        let alice = AccountId::from_seed("turns-alice");
        let bob = AccountId::from_seed("turns-bob");

        let first = turns.try_acquire(&alice).expect("first slot");
        let _second = turns.try_acquire(&alice).expect("second slot");
        assert!(turns.try_acquire(&alice).is_none(), "the cap is 2");
        assert!(
            turns.try_acquire(&bob).is_some(),
            "another account is counted separately"
        );

        drop(first);
        assert!(turns.try_acquire(&alice).is_some());
    }

    #[test]
    fn the_last_release_removes_the_accounts_entry() {
        let turns = AiTurns::new(1);
        let alice = AccountId::from_seed("turns-alice");

        drop(turns.try_acquire(&alice).expect("a slot"));

        assert!(turns.inner.running().is_empty());
    }

    #[test]
    fn a_panicking_turn_still_releases_its_permit() {
        let turns = AiTurns::new(1);
        let alice = AccountId::from_seed("turns-alice");
        let permit = turns.try_acquire(&alice).expect("a slot");

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _held = permit;
            panic!("the turn panicked");
        }));

        assert!(result.is_err());
        assert!(turns.try_acquire(&alice).is_some());
    }
}
