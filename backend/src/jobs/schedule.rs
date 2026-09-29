//! What the scheduler enqueues and how often. Each entry calls one
//! application use case that turns "what is due now" into deduplicated
//! jobs, so a tick that runs twice (or on two replicas) enqueues once.

use std::time::Duration;

use chrono::{DateTime, Utc};
use futures_util::future::BoxFuture;

use crate::application::{AppError, AppServices};

pub type TickFuture<'a> = BoxFuture<'a, Result<usize, AppError>>;

#[derive(Clone, Copy)]
pub struct Tick {
    pub name: &'static str,
    pub every: Duration,
    pub fire: for<'a> fn(&'a AppServices, DateTime<Utc>) -> TickFuture<'a>,
}

fn reminders(s: &AppServices, now: DateTime<Utc>) -> TickFuture<'_> {
    Box::pin(s.reminders.enqueue_due(now))
}

pub fn default_ticks() -> Vec<Tick> {
    // Tasks 9-10 add briefing (30 s), alerts (120 s), refresh (3600 s).
    vec![Tick {
        name: "reminders",
        every: Duration::from_secs(5),
        fire: reminders,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ticks_have_the_designed_cadences() {
        let cadences: Vec<(&str, u64)> = default_ticks()
            .iter()
            .map(|t| (t.name, t.every.as_secs()))
            .collect();
        assert_eq!(cadences, [("reminders", 5)]);
    }
}
