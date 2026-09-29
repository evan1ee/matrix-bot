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

pub fn default_ticks() -> Vec<Tick> {
    // Tasks 8-10 add reminders (5 s), briefing (30 s), alerts (120 s), refresh (3600 s).
    Vec::new()
}
