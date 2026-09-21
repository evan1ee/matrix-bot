//! What it means to create or change a todo, independent of how the request
//! arrived.
//!
//! Four transports write todos — the REST API, a Matrix command, an AI tool
//! call and an emoji reaction — and each used to carry its own copy of the
//! rules. They had drifted: only the Matrix path enforced the per-room cap,
//! only the AI path skipped the length limit entirely, only the reaction path
//! forgot to reconcile the lead-time reminder, and only the Matrix path failed
//! to announce what it had done. Every one of those was invisible from inside
//! the transport that had it right.
//!
//! So the decisions live here and the transports parse and render. This module
//! returns data and typed errors; it renders nothing.

use crate::db::DbPool;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::reminder::models::ReminderSource;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::todo::due_reminder;
use crate::modules::todo::models::{TodoItem, TodoPriority};
use crate::modules::todo::repository::TodoRepository;

/// Maximum allowed length for task description text.
pub const MAX_TASK_TEXT_LEN: usize = 500;
/// Maximum active tasks per room.
pub const MAX_ROOM_TASKS: i64 = 500;

/// A todo to create. The caller has already parsed it out of whatever it
/// arrived in.
pub struct NewTodo<'a> {
    pub text: &'a str,
    pub priority: TodoPriority,
    pub room_id: &'a str,
    pub sender_name: &'a str,
    pub due_at: Option<chrono::DateTime<chrono::Utc>>,
    pub remind_before_minutes: Option<i64>,
    /// Where a lead-time reminder should be delivered, when that is not `room_id`.
    pub remind_room_id: Option<&'a str>,
}

/// Fields to change. `None` leaves a field alone; `Some(None)` clears it.
#[derive(Default)]
pub struct TodoPatch<'a> {
    pub text: Option<&'a str>,
    pub priority: Option<TodoPriority>,
    pub completed: Option<bool>,
    pub due_at: Option<Option<chrono::DateTime<chrono::Utc>>>,
    pub remind_before_minutes: Option<Option<i64>>,
    pub remind_room_id: Option<&'a str>,
}

/// Why a write was refused.
///
/// Variants rather than a rendered string: the Matrix path answers in HTML
/// with `<code>` markup and the REST path answers in JSON, so whoever renders
/// has to be the one that knows which it is.
#[derive(Debug)]
pub enum TodoError {
    EmptyText,
    TextTooLong { max: usize },
    RoomFull { max: i64 },
    LeadOutOfRange { max: i64 },
    LeadWithoutDueDate,
    ReminderRoomRequired,
    NotFound,
    Db(anyhow::Error),
}

impl std::fmt::Display for TodoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TodoError::EmptyText => write!(f, "Task text required"),
            TodoError::TextTooLong { max } => {
                write!(f, "Task text exceeds {} character limit", max)
            }
            TodoError::RoomFull { max } => write!(
                f,
                "Room task limit reached ({} active tasks); clear completed tasks first",
                max
            ),
            TodoError::LeadOutOfRange { max } => write!(
                f,
                "remind_before_minutes must be greater than zero and at most {} (30 days)",
                max
            ),
            TodoError::LeadWithoutDueDate => {
                write!(
                    f,
                    "remind_before_minutes requires a due_at to count back from"
                )
            }
            TodoError::ReminderRoomRequired => write!(
                f,
                "remind_room_id is required for a reminder on a task that has no Matrix room"
            ),
            TodoError::NotFound => write!(f, "Task not found or access denied"),
            TodoError::Db(_) => write!(f, "Internal error"),
        }
    }
}

/// Resolves where a due reminder should be delivered.
///
/// A task created from the web carries the `web` sentinel rather than a room,
/// so the caller must name one. Local accounts share no room with the bot
/// (`api/rooms.rs` returns them an empty list by design), so they cannot.
fn reminder_room<'a>(explicit: Option<&'a str>, todo_room: &'a str) -> Result<&'a str, TodoError> {
    match explicit {
        Some(room) if !room.trim().is_empty() => Ok(room),
        _ if todo_room != "web" => Ok(todo_room),
        _ => Err(TodoError::ReminderRoomRequired),
    }
}

/// Rejects lead times that are zero, negative, or absurdly far out.
fn validate_lead(minutes: i64) -> Result<(), TodoError> {
    if minutes <= 0 || minutes > due_reminder::MAX_LEAD_MINUTES {
        return Err(TodoError::LeadOutOfRange {
            max: due_reminder::MAX_LEAD_MINUTES,
        });
    }
    Ok(())
}

/// Creates a todo: validates, writes, reconciles its reminder, then broadcasts.
///
/// Validation happens before the insert, so a rejected reminder never leaves a
/// task behind with a lead time that was silently dropped.
pub async fn create(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    input: NewTodo<'_>,
) -> Result<TodoItem, TodoError> {
    if input.text.trim().is_empty() {
        return Err(TodoError::EmptyText);
    }
    if input.text.len() > MAX_TASK_TEXT_LEN {
        return Err(TodoError::TextTooLong {
            max: MAX_TASK_TEXT_LEN,
        });
    }

    if let Some(minutes) = input.remind_before_minutes {
        validate_lead(minutes)?;
        if input.due_at.is_none() {
            return Err(TodoError::LeadWithoutDueDate);
        }
        reminder_room(input.remind_room_id, input.room_id)?;
    }

    let active = TodoRepository::count_room_active(pool, input.room_id)
        .await
        .map_err(TodoError::Db)?;
    if active >= MAX_ROOM_TASKS {
        return Err(TodoError::RoomFull {
            max: MAX_ROOM_TASKS,
        });
    }

    let item = TodoRepository::add(
        pool,
        input.room_id,
        owner_id,
        input.sender_name,
        input.text,
        input.priority,
        input.due_at,
        input.remind_before_minutes,
    )
    .await
    .map_err(TodoError::Db)?;

    // Errors here never propagate the reminder room upward — when no reminder
    // is wanted, the room named here is unused by `sync`, so falling back to
    // the task's own room (even the `web` sentinel) is harmless.
    let remind_room = reminder_room(input.remind_room_id, &item.room_id)
        .unwrap_or(&item.room_id)
        .to_string();
    due_reminder::sync(pool, broadcaster, &item, &remind_room)
        .await
        .map_err(TodoError::Db)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::TodoCreated(item.clone()));
    }

    Ok(item)
}

/// Applies a patch to an owned todo, reconciles its reminder, then broadcasts.
pub async fn update(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
    patch: TodoPatch<'_>,
) -> Result<TodoItem, TodoError> {
    let existing = TodoRepository::find_owned(pool, id, owner_id)
        .await
        .map_err(TodoError::Db)?
        .ok_or(TodoError::NotFound)?;

    if let Some(Some(minutes)) = patch.remind_before_minutes {
        validate_lead(minutes)?;
    }

    if let Some(text) = patch.text {
        TodoRepository::edit_text(pool, id, text)
            .await
            .map_err(TodoError::Db)?;
    }

    if let Some(prio) = patch.priority {
        TodoRepository::set_priority(pool, id, prio)
            .await
            .map_err(TodoError::Db)?;
    }

    if let Some(completed) = patch.completed {
        TodoRepository::set_completed(pool, id, completed)
            .await
            .map_err(TodoError::Db)?;
    }

    if let Some(due) = patch.due_at {
        TodoRepository::set_due_date(pool, id, due)
            .await
            .map_err(TodoError::Db)?;
    }

    if let Some(minutes) = patch.remind_before_minutes {
        TodoRepository::set_remind_before(pool, id, minutes)
            .await
            .map_err(TodoError::Db)?;
    }

    // Re-read rather than patching the in-memory copy: the reminder is
    // reconciled from stored state, so the two must agree.
    let item = TodoRepository::find(pool, id)
        .await
        .map_err(TodoError::Db)?
        .ok_or(TodoError::NotFound)?;

    if due_reminder::wanted_trigger(&item).is_some() {
        // Keep delivering wherever the existing reminder already goes unless
        // the caller names a new room. Without this fallback, editing just the
        // text of a web task that has a reminder would fail for want of a room
        // the caller never intended to change.
        let current_room =
            ReminderRepository::find_pending_for_source(pool, ReminderSource::Todo, id)
                .await
                .map_err(TodoError::Db)?
                .map(|r| r.room_id);

        let remind_room = match patch.remind_room_id {
            Some(room) if !room.trim().is_empty() => room.to_string(),
            _ => match current_room {
                Some(room) => room,
                None => reminder_room(None, &item.room_id)?.to_string(),
            },
        };

        due_reminder::sync(pool, broadcaster, &item, &remind_room)
            .await
            .map_err(TodoError::Db)?;
    } else if due_reminder::wanted_trigger(&existing).is_some() {
        // The task no longer wants a reminder but used to — tear the old one down.
        due_reminder::tear_down(pool, broadcaster, id)
            .await
            .map_err(TodoError::Db)?;
    }

    if let Some(b) = broadcaster {
        b.send(SystemEvent::TodoUpdated(item.clone()));
    }

    Ok(item)
}

/// Deletes an owned todo, tearing down its reminder first — a reminder whose
/// task no longer exists would fire pointing at nothing.
pub async fn delete(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id: i64,
) -> Result<TodoItem, TodoError> {
    let item = TodoRepository::find_owned(pool, id, owner_id)
        .await
        .map_err(TodoError::Db)?
        .ok_or(TodoError::NotFound)?;

    due_reminder::tear_down(pool, broadcaster, id)
        .await
        .map_err(TodoError::Db)?;

    TodoRepository::delete(pool, id)
        .await
        .map_err(TodoError::Db)?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::TodoDeleted {
            id,
            room_id: item.room_id.clone(),
            owner_id: owner_id.to_string(),
        });
    }

    Ok(item)
}

/// Clears completed tasks — for one room when `room_id` is `Some`, else for
/// the owner across every room — broadcasting one `TodoDeleted` per row
/// removed and returning the rows so a caller can name what went away.
pub async fn clear_completed(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: Option<&str>,
) -> Result<Vec<TodoItem>, TodoError> {
    let items = match room_id {
        Some(room) => TodoRepository::list_completed_room(pool, room)
            .await
            .map_err(TodoError::Db)?,
        None => TodoRepository::list_completed_user(pool, owner_id)
            .await
            .map_err(TodoError::Db)?,
    };

    match room_id {
        Some(room) => {
            TodoRepository::clear_completed_room(pool, room)
                .await
                .map_err(TodoError::Db)?;
        }
        None => {
            TodoRepository::clear_completed_user(pool, owner_id)
                .await
                .map_err(TodoError::Db)?;
        }
    }

    if let Some(b) = broadcaster {
        for item in &items {
            b.send(SystemEvent::TodoDeleted {
                id: item.id,
                room_id: item.room_id.clone(),
                owner_id: item.owner_id.clone(),
            });
        }
    }

    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use chrono::{Duration, Utc};
    use sqlx::SqlitePool;

    async fn setup() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    fn new_todo(text: &str) -> NewTodo<'_> {
        NewTodo {
            text,
            priority: TodoPriority::Medium,
            room_id: "!room:1",
            sender_name: "alice",
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
        }
    }

    /// Every rejection `create` can produce, in one place — this replaces
    /// per-transport validation tests. Each transport keeps only a smoke test
    /// that a rejection surfaces in its own format.
    #[tokio::test]
    async fn test_create_rejection_parity_table() {
        let pool = setup().await;
        let due = Some(Utc::now() + Duration::hours(6));
        let over_limit_text = "x".repeat(MAX_TASK_TEXT_LEN + 1);

        struct Case<'a> {
            name: &'a str,
            input: NewTodo<'a>,
            expect: fn(&TodoError) -> bool,
        }

        let cases = vec![
            Case {
                name: "empty text",
                input: new_todo("   "),
                expect: |e| matches!(e, TodoError::EmptyText),
            },
            Case {
                name: "text one character over the limit",
                input: new_todo(&over_limit_text),
                expect: |e| matches!(e, TodoError::TextTooLong { .. }),
            },
            Case {
                name: "lead time of 0",
                input: NewTodo {
                    remind_before_minutes: Some(0),
                    due_at: due,
                    ..new_todo("Ship it")
                },
                expect: |e| matches!(e, TodoError::LeadOutOfRange { .. }),
            },
            Case {
                name: "negative lead",
                input: NewTodo {
                    remind_before_minutes: Some(-5),
                    due_at: due,
                    ..new_todo("Ship it")
                },
                expect: |e| matches!(e, TodoError::LeadOutOfRange { .. }),
            },
            Case {
                name: "lead over MAX_LEAD_MINUTES",
                input: NewTodo {
                    remind_before_minutes: Some(due_reminder::MAX_LEAD_MINUTES + 1),
                    due_at: due,
                    ..new_todo("Ship it")
                },
                expect: |e| matches!(e, TodoError::LeadOutOfRange { .. }),
            },
            Case {
                name: "lead with no due date",
                input: NewTodo {
                    remind_before_minutes: Some(60),
                    due_at: None,
                    ..new_todo("Ship it")
                },
                expect: |e| matches!(e, TodoError::LeadWithoutDueDate),
            },
            Case {
                name: "reminder room of web",
                input: NewTodo {
                    room_id: "web",
                    remind_before_minutes: Some(60),
                    due_at: due,
                    remind_room_id: None,
                    ..new_todo("Ship it")
                },
                expect: |e| matches!(e, TodoError::ReminderRoomRequired),
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

        // A room at MAX_ROOM_TASKS is a distinct case: it depends on prior
        // state, not just the input, so it cannot share the table above.
        let full_room = "!room:full";
        for i in 0..MAX_ROOM_TASKS {
            create(
                &pool,
                None,
                "@alice:a",
                NewTodo {
                    room_id: full_room,
                    ..new_todo(&format!("task {i}"))
                },
            )
            .await
            .unwrap();
        }
        let err = create(
            &pool,
            None,
            "@alice:a",
            NewTodo {
                room_id: full_room,
                ..new_todo("one too many")
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, TodoError::RoomFull { .. }));
    }

    /// Text exactly at the limit must be accepted; one character over must not.
    #[tokio::test]
    async fn test_text_length_boundary() {
        let pool = setup().await;

        let at_limit = "x".repeat(MAX_TASK_TEXT_LEN);
        assert!(create(&pool, None, "@alice:a", new_todo(&at_limit))
            .await
            .is_ok());

        let over_limit = "x".repeat(MAX_TASK_TEXT_LEN + 1);
        let err = create(&pool, None, "@alice:a", new_todo(&over_limit))
            .await
            .unwrap_err();
        assert!(matches!(err, TodoError::TextTooLong { max } if max == MAX_TASK_TEXT_LEN));
    }

    /// The bug this whole module exists to fix: clearing completed tasks left
    /// every open browser showing rows that were already gone.
    #[tokio::test]
    async fn test_clearing_completed_announces_each_removal() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let mut ids = Vec::new();
        for text in ["one", "two", "three"] {
            let item = create(&pool, Some(&broadcaster), "@alice:a", new_todo(text))
                .await
                .unwrap();
            ids.push(item.id);
        }
        for id in &ids {
            update(
                &pool,
                Some(&broadcaster),
                "@alice:a",
                *id,
                TodoPatch {
                    completed: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        }
        // Drain the Created/Updated events from setup so only clear's events remain.
        while rx.try_recv().is_ok() {}

        let cleared = clear_completed(&pool, Some(&broadcaster), "@alice:a", None)
            .await
            .unwrap();
        assert_eq!(cleared.len(), 3);

        let mut seen = 0;
        while let Ok(event) = rx.try_recv() {
            if let SystemEvent::TodoDeleted { id, .. } = event {
                assert!(ids.contains(&id));
                seen += 1;
            }
        }
        assert_eq!(seen, 3, "one TodoDeleted must be broadcast per cleared row");
    }
}
