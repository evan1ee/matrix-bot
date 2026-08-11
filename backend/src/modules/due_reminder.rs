//! Keeps a lead-time reminder in step with the row that scheduled it.
//!
//! A row that owns a due date and a lead time — a todo, a recurring bill —
//! owns at most one pending row in `reminders`, linked by
//! `(source_kind, source_id)`. Every path that can change a due date, a lead
//! time, or whether the row still wants reminding calls [`sync`], which
//! reconciles that row: creating, rescheduling, or cancelling as needed.
//!
//! Reconciling from current state rather than patching per-edit means the two
//! can never drift, whichever path made the change. Modules describe what they
//! want as a [`ReminderPlan`] and share this one reconciler, so a second module
//! wanting due reminders does not mean a second copy of these rules.

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::reminder::models::{RecurrencePattern, ReminderSource};
use crate::modules::reminder::repository::ReminderRepository;

/// Longest lead time accepted, so a typo cannot schedule a reminder years out.
pub const MAX_LEAD_MINUTES: i64 = 60 * 24 * 30;

/// What [`sync`] did, so callers can tell the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    /// No reminder wanted and none existed.
    Nothing,
    /// A reminder is now pending at this instant.
    Scheduled(DateTime<Utc>),
    /// The reminder was torn down — the due date, the lead time, or the row
    /// itself went away.
    Cancelled,
    /// The lead time lands in the past, so nothing was scheduled. Firing
    /// immediately would be noise, and silently ignoring it would be a lie.
    LeadTimeInPast(DateTime<Utc>),
}

/// What a module wants reminding about.
///
/// `trigger_at` is `None` when no reminder is wanted at all — the row was
/// completed or paid off, its lead time was cleared, or it never had a due
/// date. That is what makes finishing something tear its reminder down.
pub struct ReminderPlan<'a> {
    pub source: ReminderSource,
    pub source_id: i64,
    pub owner_id: &'a str,
    pub owner_name: &'a str,
    /// Where the reminder should be delivered — the originating Matrix room, or
    /// the room a web caller picked. A row carrying the `web` sentinel has no
    /// room of its own, so the caller supplies one.
    pub room_id: &'a str,
    pub message: String,
    pub trigger_at: Option<DateTime<Utc>>,
}

/// Reconciles the linked reminder against the plan's current state.
///
/// `broadcaster` is optional because the Matrix command path has no access to
/// one; see the note in `modules/todo/handler.rs`.
pub async fn sync(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    plan: &ReminderPlan<'_>,
) -> Result<SyncOutcome> {
    let existing =
        ReminderRepository::find_pending_for_source(pool, plan.source, plan.source_id).await?;

    let Some(trigger_at) = plan.trigger_at else {
        return tear_down(pool, broadcaster, plan.source, plan.source_id).await;
    };

    if trigger_at <= Utc::now() {
        // Drop any stale reminder before reporting, so a due date pushed into
        // the past does not leave one pending behind it.
        tear_down(pool, broadcaster, plan.source, plan.source_id).await?;
        return Ok(SyncOutcome::LeadTimeInPast(trigger_at));
    }

    if let Some(current) = &existing {
        if current.trigger_at == trigger_at
            && current.room_id == plan.room_id
            && current.message == plan.message
        {
            return Ok(SyncOutcome::Scheduled(trigger_at));
        }
        // Time, destination, or wording moved. Replacing rather than patching
        // keeps one creation path, so a rescheduled reminder cannot end up
        // half-updated.
        tear_down(pool, broadcaster, plan.source, plan.source_id).await?;
    }

    let reminder = ReminderRepository::create(
        pool,
        plan.room_id,
        plan.owner_id,
        plan.owner_name,
        &plan.message,
        trigger_at,
        RecurrencePattern::None,
        Some((plan.source, plan.source_id)),
    )
    .await?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderCreated(reminder));
    }

    Ok(SyncOutcome::Scheduled(trigger_at))
}

/// Cancels the linked reminder if one is pending, broadcasting the change.
pub async fn tear_down(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    source: ReminderSource,
    source_id: i64,
) -> Result<SyncOutcome> {
    let Some(cancelled) = ReminderRepository::cancel_for_source(pool, source, source_id).await?
    else {
        return Ok(SyncOutcome::Nothing);
    };

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderCancelled {
            id: cancelled.id,
            room_id: cancelled.room_id,
            owner_id: cancelled.owner_id,
        });
    }

    Ok(SyncOutcome::Cancelled)
}
