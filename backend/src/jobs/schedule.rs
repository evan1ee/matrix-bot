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

fn briefing(s: &AppServices, now: DateTime<Utc>) -> TickFuture<'_> {
    Box::pin(s.briefing.enqueue_due(now))
}

fn portfolio_alerts(s: &AppServices, now: DateTime<Utc>) -> TickFuture<'_> {
    Box::pin(s.portfolio.enqueue_alert_check(now))
}

fn portfolio_refresh(s: &AppServices, now: DateTime<Utc>) -> TickFuture<'_> {
    Box::pin(s.portfolio.enqueue_price_refresh(now))
}

fn agent_scheduled_tasks(s: &AppServices, _now: DateTime<Utc>) -> TickFuture<'_> {
    Box::pin(s.agent.enqueue_due_scheduled_tasks())
}

pub fn default_ticks() -> Vec<Tick> {
    vec![
        Tick {
            name: "reminders",
            every: Duration::from_secs(5),
            fire: reminders,
        },
        Tick {
            name: "briefing",
            every: Duration::from_secs(30),
            fire: briefing,
        },
        Tick {
            name: "portfolio_alerts",
            every: Duration::from_secs(120),
            fire: portfolio_alerts,
        },
        Tick {
            name: "portfolio_refresh",
            every: Duration::from_secs(3_600),
            fire: portfolio_refresh,
        },
        Tick {
            name: "agent_scheduled_tasks",
            every: Duration::from_secs(60),
            fire: agent_scheduled_tasks,
        },
    ]
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
        assert_eq!(
            cadences,
            [
                ("reminders", 5),
                ("briefing", 30),
                ("portfolio_alerts", 120),
                ("portfolio_refresh", 3_600),
                ("agent_scheduled_tasks", 60),
            ]
        );
    }
}
