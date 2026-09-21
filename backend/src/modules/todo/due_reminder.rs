//! Keeps a todo's lead-time reminder in step with the todo.
//!
//! The reconcile rules live in [`crate::modules::due_reminder`], shared with
//! every other module that owns a due date. This module's job is only to say
//! what a todo wants: when it should fire, and what it should say.

use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};

use crate::events::EventBroadcaster;
use crate::modules::due_reminder::{self, ReminderPlan};

pub use crate::modules::due_reminder::SyncOutcome;
use crate::modules::reminder::models::ReminderSource;
use crate::modules::todo::models::TodoItem;

pub use crate::modules::due_reminder::MAX_LEAD_MINUTES;

/// The instant a reminder should fire for `todo`, if it wants one at all.
///
/// A completed task never wants one, which is what makes completing a task tear
/// its reminder down.
pub fn wanted_trigger(todo: &TodoItem) -> Option<DateTime<Utc>> {
    match (todo.due_at, todo.remind_before_minutes, todo.completed) {
        (Some(due), Some(minutes), false) if minutes > 0 => Some(due - Duration::minutes(minutes)),
        _ => None,
    }
}

/// Reconciles the linked reminder against the todo's current state.
///
/// `room_id` is where the reminder should be delivered — the originating Matrix
/// room, or the room the web caller picked. A todo carrying the `web` sentinel
/// has no room of its own, so the caller supplies one.
pub async fn sync(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    todo: &TodoItem,
    room_id: &str,
) -> Result<SyncOutcome> {
    let plan = ReminderPlan {
        source: ReminderSource::Todo,
        source_id: todo.id,
        owner_id: &todo.owner_id,
        owner_name: &todo.sender_name,
        room_id,
        message: format!("Task #{} is due: {}", todo.id, todo.text),
        trigger_at: wanted_trigger(todo),
    };

    due_reminder::sync(pool, broadcaster, &plan).await
}

/// Cancels the todo's linked reminder if one is pending.
pub async fn tear_down(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    todo_id: i64,
) -> Result<SyncOutcome> {
    due_reminder::tear_down(pool, broadcaster, ReminderSource::Todo, todo_id).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::modules::reminder::repository::ReminderRepository;
    use crate::modules::todo::models::TodoPriority;
    use crate::modules::todo::repository::TodoRepository;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        pool
    }

    async fn todo_with(
        pool: &DbPool,
        due_at: Option<DateTime<Utc>>,
        lead: Option<i64>,
    ) -> TodoItem {
        TodoRepository::add(
            pool,
            "web",
            "@alice:a",
            "alice",
            "Ship it",
            TodoPriority::Medium,
            due_at,
            lead,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_schedules_at_due_minus_lead() {
        let pool = setup().await;
        let due = crate::utils::time::now_at_storage_precision() + Duration::hours(4);
        let todo = todo_with(&pool, Some(due), Some(60)).await;

        let outcome = sync(&pool, None, &todo, "!room:a").await.unwrap();

        let expected = due - Duration::minutes(60);
        assert_eq!(outcome, SyncOutcome::Scheduled(expected));

        let rem = ReminderRepository::find_pending_for_source(&pool, ReminderSource::Todo, todo.id)
            .await
            .unwrap()
            .expect("a reminder must be pending");
        assert_eq!(rem.trigger_at, expected);
        assert_eq!(rem.room_id, "!room:a");
        assert_eq!(rem.source_kind.as_deref(), Some("todo"));
        assert_eq!(rem.source_id, Some(todo.id));
    }

    #[tokio::test]
    async fn test_no_due_date_or_no_lead_schedules_nothing() {
        let pool = setup().await;

        let no_due = todo_with(&pool, None, Some(60)).await;
        assert_eq!(
            sync(&pool, None, &no_due, "!room:a").await.unwrap(),
            SyncOutcome::Nothing
        );

        let no_lead = todo_with(
            &pool,
            Some(crate::utils::time::now_at_storage_precision() + Duration::hours(4)),
            None,
        )
        .await;
        assert_eq!(
            sync(&pool, None, &no_lead, "!room:a").await.unwrap(),
            SyncOutcome::Nothing
        );
    }

    #[tokio::test]
    async fn test_lead_time_already_past_schedules_nothing() {
        let pool = setup().await;
        // Due in 10 minutes, but asking for a reminder an hour beforehand.
        let due = crate::utils::time::now_at_storage_precision() + Duration::minutes(10);
        let todo = todo_with(&pool, Some(due), Some(60)).await;

        let outcome = sync(&pool, None, &todo, "!room:a").await.unwrap();

        assert!(matches!(outcome, SyncOutcome::LeadTimeInPast(_)));
        assert!(
            ReminderRepository::find_pending_for_source(&pool, ReminderSource::Todo, todo.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_completing_the_task_cancels_the_reminder() {
        let pool = setup().await;
        let todo = todo_with(
            &pool,
            Some(crate::utils::time::now_at_storage_precision() + Duration::hours(4)),
            Some(60),
        )
        .await;
        sync(&pool, None, &todo, "!room:a").await.unwrap();

        let done = TodoRepository::mark_done(&pool, todo.id)
            .await
            .unwrap()
            .unwrap();
        let outcome = sync(&pool, None, &done, "!room:a").await.unwrap();

        assert_eq!(outcome, SyncOutcome::Cancelled);
        assert!(
            ReminderRepository::find_pending_for_source(&pool, ReminderSource::Todo, todo.id)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn test_moving_the_due_date_reschedules_without_stacking() {
        let pool = setup().await;
        let todo = todo_with(
            &pool,
            Some(crate::utils::time::now_at_storage_precision() + Duration::hours(4)),
            Some(60),
        )
        .await;
        sync(&pool, None, &todo, "!room:a").await.unwrap();

        let moved_due = crate::utils::time::now_at_storage_precision() + Duration::days(2);
        let moved = TodoRepository::set_due_date(&pool, todo.id, Some(moved_due))
            .await
            .unwrap()
            .unwrap();
        let outcome = sync(&pool, None, &moved, "!room:a").await.unwrap();

        assert_eq!(
            outcome,
            SyncOutcome::Scheduled(moved_due - Duration::minutes(60))
        );

        let pending: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM reminders WHERE source_kind = 'todo' AND source_id = ? \
                 AND status = 'pending'",
            todo.id
        )
        .unwrap();
        assert_eq!(pending, 1, "rescheduling must replace, not stack");
    }

    #[tokio::test]
    async fn test_clearing_the_lead_time_cancels_the_reminder() {
        let pool = setup().await;
        let todo = todo_with(
            &pool,
            Some(crate::utils::time::now_at_storage_precision() + Duration::hours(4)),
            Some(60),
        )
        .await;
        sync(&pool, None, &todo, "!room:a").await.unwrap();

        let cleared = TodoRepository::set_remind_before(&pool, todo.id, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            sync(&pool, None, &cleared, "!room:a").await.unwrap(),
            SyncOutcome::Cancelled
        );
    }

    /// Re-syncing an unchanged todo must be a no-op, not a churn of
    /// cancel/create pairs — the SSE stream would flicker otherwise.
    #[tokio::test]
    async fn test_sync_is_idempotent() {
        let pool = setup().await;
        let todo = todo_with(
            &pool,
            Some(crate::utils::time::now_at_storage_precision() + Duration::hours(4)),
            Some(60),
        )
        .await;

        let first = sync(&pool, None, &todo, "!room:a").await.unwrap();
        let second = sync(&pool, None, &todo, "!room:a").await.unwrap();
        assert_eq!(first, second);

        let total: i64 = crate::db_scalar_one!(
            &pool,
            _,
            "SELECT COUNT(*) FROM reminders WHERE source_kind = 'todo' AND source_id = ?",
            todo.id
        )
        .unwrap();
        assert_eq!(total, 1);
    }
}
