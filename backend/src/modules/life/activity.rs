//! What happened to one owner on one day, gathered from every module.
//!
//! This is domain logic, not transport: the Matrix `!journal` command and the
//! REST day view both ask the same question and must get the same answer. It
//! lived under `api/` and was imported back into a bot module, which is the
//! dependency this file exists to remove.

use crate::db::DbPool;
use serde::Serialize;
use utoipa::ToSchema;

use crate::modules::life::activity_collectors::*;

/// A single event from any module that happened on a given date.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ActivityEvent {
    /// ISO time within the day, e.g. "09:15:00" (or null if date-only)
    pub time: Option<String>,
    /// Source module identifier
    pub module: &'static str,
    /// Verb: "created", "completed", "expense", "income", "buy", "sell", etc.
    pub action: String,
    /// Visual emoji for the event type
    pub emoji: &'static str,
    /// One-line human summary
    pub summary: String,
    /// Optional secondary detail
    pub detail: Option<String>,
}

/// Summary counts per module.
#[derive(Debug, Clone, Serialize, Default, ToSchema)]
pub struct ActivityStats {
    pub total_events: usize,
    pub todos_created: usize,
    pub todos_completed: usize,
    pub todos_due: usize,
    pub expenses: usize,
    pub income: usize,
    pub portfolio_trades: usize,
    pub habits_done: usize,
    pub reminders_fired: usize,
    pub bookmarks_saved: usize,
    pub bills_paid: usize,
    pub footprints: usize,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DayActivityResponse {
    pub date: String,
    pub events: Vec<ActivityEvent>,
    pub stats: ActivityStats,
}

pub async fn collect_day_activity(pool: &DbPool, owner: &str, date: &str) -> DayActivityResponse {
    let mut events: Vec<ActivityEvent> = Vec::new();
    let mut stats = ActivityStats::default();

    // --- Todos ---
    query_todos(pool, owner, date, &mut events, &mut stats).await;

    // --- Finance Transactions ---
    query_finance(pool, owner, date, &mut events, &mut stats).await;

    // --- Portfolio Audit Log ---
    query_portfolio(pool, owner, date, &mut events, &mut stats).await;

    // --- Habits ---
    query_habits(pool, owner, date, &mut events, &mut stats).await;

    // --- Reminders ---
    query_reminders(pool, owner, date, &mut events, &mut stats).await;

    // --- Bookmarks ---
    query_bookmarks(pool, owner, date, &mut events, &mut stats).await;

    // --- Footprints ---
    query_footprints(pool, owner, date, &mut events, &mut stats).await;

    // Sort chronologically (nulls last)
    events.sort_by(|a, b| a.time.cmp(&b.time));
    stats.total_events = events.len();

    DayActivityResponse {
        date: date.to_string(),
        events,
        stats,
    }
}
