//! What it means to set, cancel, reschedule or mark a reminder triggered,
//! independent of how the request arrived.
//!
//! Four transports touch reminders — a Matrix command, the REST API, an AI
//! tool call and an emoji reaction — and they had drifted: the REST path
//! enforced a 500-character message limit while the Matrix path enforced a
//! different, shorter one, and only the Matrix module actually dropped its
//! broadcaster outright (it named the parameter `_broadcaster`), so every
//! reminder set over Matrix was invisible to an open browser until the page
//! was reloaded.
//!
//! So the decisions live here and the transports parse and render. This
//! module returns data and typed errors; it renders nothing.
//!
//! `modules::due_reminder` and `modules::todo::due_reminder` are a different
//! thing entirely: the reconciler that keeps a lead-time reminder in step
//! with the task or bill that scheduled it. `modules::reminder::scheduler` is
//! the delivery loop that fires reminders and marks them triggered as they
//! go. Neither is a transport handling a user request, so neither routes
//! through here.

use crate::db::DbPool;
use chrono::{DateTime, Utc};

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::reminder::models::{RecurrencePattern, Reminder};
use crate::modules::reminder::repository::ReminderRepository;

/// Maximum allowed length for a reminder message.
///
/// `api/reminders.rs` and `modules/reminder/handler.rs` disagreed before this
/// existed — 500 characters on the REST side, 300 on the Matrix side. The REST
/// limit wins here; see the Part A report for what that changes on the Matrix
/// side.
pub const MAX_MESSAGE_LEN: usize = 500;

/// A reminder to create. The caller has already parsed it out of whatever it
/// arrived in.
pub struct NewReminder<'a> {
    pub message: &'a str,
    pub trigger_at: DateTime<Utc>,
    pub room_id: &'a str,
    pub owner_name: &'a str,
    pub recurrence: RecurrencePattern,
}

/// Why a write was refused.
///
/// Variants rather than a rendered string: the Matrix path answers in HTML
/// with `<code>` markup and the REST path answers in JSON, so whoever renders
/// has to be the one that knows which it is.
#[derive(Debug)]
pub enum ReminderError {
    EmptyMessage,
    MessageTooLong {
        max: usize,
    },
    /// A trigger time that has already passed.
    TriggerInPast,
    NotFound,
    Db(anyhow::Error),
}

impl std::fmt::Display for ReminderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReminderError::EmptyMessage => write!(f, "Message required"),
            ReminderError::MessageTooLong { max } => {
                write!(f, "Message exceeds {} character limit", max)
            }
            ReminderError::TriggerInPast => write!(f, "Trigger time must be in the future"),
            ReminderError::NotFound => write!(f, "Reminder not found or access denied"),
            ReminderError::Db(_) => write!(f, "Internal error"),
        }
    }
}

fn validate(message: &str, trigger_at: DateTime<Utc>) -> Result<(), ReminderError> {
    if message.trim().is_empty() {
        return Err(ReminderError::EmptyMessage);
    }
    if message.len() > MAX_MESSAGE_LEN {
        return Err(ReminderError::MessageTooLong {
            max: MAX_MESSAGE_LEN,
        });
    }
    if trigger_at <= Utc::now() {
        return Err(ReminderError::TriggerInPast);
    }
    Ok(())
}

/// Creates a reminder: validates, writes, then broadcasts.
pub async fn create(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    input: NewReminder<'_>,
) -> Result<Reminder, ReminderError> {
    validate(input.message, input.trigger_at)?;

    let reminder = ReminderRepository::create(
        pool,
        input.room_id,
        owner_id,
        input.owner_name,
        input.message,
        input.trigger_at,
        input.recurrence,
        None,
    )
    .await
    .map_err(ReminderError::Db)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderCreated(reminder.clone()));
    }

    Ok(reminder)
}

/// Cancels an owned, still-pending reminder, then broadcasts.
pub async fn cancel(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
) -> Result<Reminder, ReminderError> {
    let reminder = ReminderRepository::find_owned(pool, owner_id, id)
        .await
        .map_err(ReminderError::Db)?
        .ok_or(ReminderError::NotFound)?;

    let cancelled = ReminderRepository::cancel(pool, owner_id, id)
        .await
        .map_err(ReminderError::Db)?;
    if !cancelled {
        // Owned but not pending (already triggered/cancelled/failed): cancel
        // is not a valid move on it, so this is a not-found from the caller's
        // point of view.
        return Err(ReminderError::NotFound);
    }

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderCancelled {
            id,
            room_id: reminder.room_id.clone(),
            owner_id: owner_id.to_string(),
        });
    }

    Ok(reminder)
}

/// Moves an owned, still-pending reminder's fire time, keeping its message and
/// recurrence, then broadcasts.
///
/// This is the "+1 Day" reaction's operation, not the REST `PATCH` endpoint's
/// full replace (message, fire time and recurrence together) — that stays on
/// `ReminderRepository::update_pending` directly in `api/reminders.rs`; see
/// the Part A report for why.
pub async fn reschedule(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
    trigger_at: DateTime<Utc>,
) -> Result<Reminder, ReminderError> {
    let existing = ReminderRepository::find_owned(pool, owner_id, id)
        .await
        .map_err(ReminderError::Db)?
        .ok_or(ReminderError::NotFound)?;

    ReminderRepository::update_pending(
        pool,
        owner_id,
        id,
        &existing.message,
        trigger_at,
        RecurrencePattern::from_str_or_default(&existing.recurrence),
    )
    .await
    .map_err(ReminderError::Db)?
    .ok_or(ReminderError::NotFound)?;

    let updated = ReminderRepository::find_owned(pool, owner_id, id)
        .await
        .map_err(ReminderError::Db)?
        .ok_or(ReminderError::NotFound)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderUpdated(updated.clone()));
    }

    Ok(updated)
}

/// Marks an owned reminder as triggered, as if it had just fired, then
/// broadcasts. Used by a manual "done" or "celebrate" reaction.
///
/// The scheduler's own delivery loop calls `ReminderRepository::mark_triggered`
/// directly — marking a reminder triggered as it actually fires is not a user
/// request, so it does not route through here.
pub async fn mark_triggered(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
) -> Result<Reminder, ReminderError> {
    ReminderRepository::find_owned(pool, owner_id, id)
        .await
        .map_err(ReminderError::Db)?
        .ok_or(ReminderError::NotFound)?;

    ReminderRepository::mark_triggered(pool, id)
        .await
        .map_err(ReminderError::Db)?;

    let updated = ReminderRepository::find_owned(pool, owner_id, id)
        .await
        .map_err(ReminderError::Db)?
        .ok_or(ReminderError::NotFound)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderUpdated(updated.clone()));
    }

    Ok(updated)
}

/// Clears the owner's reminder history (everything not pending), then
/// broadcasts.
pub async fn clear_history(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
) -> Result<(), ReminderError> {
    ReminderRepository::clear_user_history(pool, owner_id)
        .await
        .map_err(ReminderError::Db)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::ReminderHistoryCleared {
            owner_id: owner_id.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use chrono::Duration;
    use sqlx::SqlitePool;

    async fn setup() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    fn new_reminder(message: &str, trigger_at: DateTime<Utc>) -> NewReminder<'_> {
        NewReminder {
            message,
            trigger_at,
            room_id: "!room:1",
            owner_name: "alice",
            recurrence: RecurrencePattern::None,
        }
    }

    /// Every rejection `create` can produce, in one place — this replaces
    /// per-transport validation tests. Each transport keeps only a smoke test
    /// that a rejection surfaces in its own format.
    #[tokio::test]
    async fn test_create_rejection_parity_table() {
        let pool = setup().await;
        let future = Utc::now() + Duration::hours(1);
        let past = Utc::now() - Duration::hours(1);
        let over_limit_message = "x".repeat(MAX_MESSAGE_LEN + 1);

        struct Case<'a> {
            name: &'a str,
            input: NewReminder<'a>,
            expect: fn(&ReminderError) -> bool,
        }

        let cases = vec![
            Case {
                name: "empty message",
                input: new_reminder("   ", future),
                expect: |e| matches!(e, ReminderError::EmptyMessage),
            },
            Case {
                name: "message one character over the limit",
                input: new_reminder(&over_limit_message, future),
                expect: |e| matches!(e, ReminderError::MessageTooLong { .. }),
            },
            Case {
                name: "trigger time in the past",
                input: new_reminder("Check oven", past),
                expect: |e| matches!(e, ReminderError::TriggerInPast),
            },
        ];

        for case in cases {
            let err = create(&pool, None, "@alice:a", case.input)
                .await
                .expect_err(&format!("{} must be rejected", case.name));
            assert!(
                (case.expect)(&err),
                "{}: unexpected error variant {:?}",
                case.name,
                err
            );
        }
    }

    /// The bug this whole module exists to fix, for the Matrix path: a
    /// reminder set over Matrix never reached an open browser because the
    /// module never broadcast anything.
    #[tokio::test]
    async fn test_creating_a_reminder_announces_itself() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let rem = create(
            &pool,
            Some(&broadcaster),
            "@alice:a",
            new_reminder("Check oven", Utc::now() + Duration::hours(1)),
        )
        .await
        .unwrap();

        let event = rx.try_recv().expect("create must broadcast an event");
        match event {
            SystemEvent::ReminderCreated(created) => assert_eq!(created.id, rem.id),
            other => panic!("unexpected event: {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_cancel_is_owner_scoped() {
        let pool = setup().await;
        let rem = create(
            &pool,
            None,
            "@alice:a",
            new_reminder("Check oven", Utc::now() + Duration::hours(1)),
        )
        .await
        .unwrap();

        let err = cancel(&pool, None, "@mallory:a", rem.id).await.unwrap_err();
        assert!(matches!(err, ReminderError::NotFound));

        let cancelled = cancel(&pool, None, "@alice:a", rem.id).await.unwrap();
        assert_eq!(cancelled.id, rem.id);

        // Already cancelled: a second cancel finds nothing left to do.
        let err = cancel(&pool, None, "@alice:a", rem.id).await.unwrap_err();
        assert!(matches!(err, ReminderError::NotFound));
    }

    #[tokio::test]
    async fn test_reschedule_moves_the_trigger_time_and_keeps_the_message() {
        let pool = setup().await;
        let original = Utc::now() + Duration::hours(1);
        let rem = create(
            &pool,
            None,
            "@alice:a",
            new_reminder("Check oven", original),
        )
        .await
        .unwrap();

        let new_trigger = original + Duration::days(1);
        let updated = reschedule(&pool, None, "@alice:a", rem.id, new_trigger)
            .await
            .unwrap();

        assert_eq!(updated.message, "Check oven");
        assert!(updated.trigger_at > original);

        let err = reschedule(&pool, None, "@mallory:a", rem.id, new_trigger)
            .await
            .unwrap_err();
        assert!(matches!(err, ReminderError::NotFound));
    }

    #[tokio::test]
    async fn test_mark_triggered_is_owner_scoped_and_announces_itself() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let rem = create(
            &pool,
            None,
            "@alice:a",
            new_reminder("Check oven", Utc::now() + Duration::hours(1)),
        )
        .await
        .unwrap();

        let err = mark_triggered(&pool, None, "@mallory:a", rem.id)
            .await
            .unwrap_err();
        assert!(matches!(err, ReminderError::NotFound));

        let updated = mark_triggered(&pool, Some(&broadcaster), "@alice:a", rem.id)
            .await
            .unwrap();
        assert_eq!(updated.status, "triggered");

        let event = rx.try_recv().expect("mark_triggered must broadcast");
        assert!(matches!(event, SystemEvent::ReminderUpdated(_)));
    }

    #[tokio::test]
    async fn test_clear_history_announces_itself() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let rem = create(
            &pool,
            None,
            "@alice:a",
            new_reminder("Check oven", Utc::now() + Duration::hours(1)),
        )
        .await
        .unwrap();
        cancel(&pool, None, "@alice:a", rem.id).await.unwrap();
        while rx.try_recv().is_ok() {}

        clear_history(&pool, Some(&broadcaster), "@alice:a")
            .await
            .unwrap();

        let event = rx.try_recv().expect("clear_history must broadcast");
        assert!(matches!(event, SystemEvent::ReminderHistoryCleared { .. }));
    }
}
