//! Locked completion of todos and the successor a recurring todo spawns.
//!
//! Both entry points (`set_completed` and an `update` whose patch completes the
//! todo) lock the row `FOR UPDATE` first, so concurrent or repeated completions
//! serialise: the second sees the todo already completed, and a todo whose
//! `next_todo_id` is set never spawns again (done → undo → done).

use chrono::Utc;
use sqlx::{Postgres, Transaction};

use super::TodoService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::reminders::{ReminderSource, SyncOutcome};
use crate::domain::todos::{successor, validate_merged, TodoItem, TodoPatch};
use crate::infrastructure::postgres::repositories::{self, outbox};

/// A completion change and, when it spawned one, the next occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionOutcome {
    pub todo: TodoItem,
    pub next: Option<TodoItem>,
}

fn not_found() -> AppError {
    AppError::NotFound("Todo item not found or access denied".to_string())
}

impl TodoService {
    pub async fn set_completed(
        &self,
        owner_id: &AccountId,
        id: i64,
        completed: bool,
    ) -> Result<CompletionOutcome, AppError> {
        let mut tx = self.pool.begin().await?;
        let current = repositories::todos::lock_owned(&mut *tx, owner_id, id)
            .await?
            .ok_or_else(not_found)?;
        if current.completed == completed {
            return Ok(CompletionOutcome {
                todo: current,
                next: None,
            });
        }

        if completed {
            let outcome = complete_locked(&mut tx, owner_id, current).await?;
            tx.commit().await?;
            self.after_completion(&outcome).await?;
            return Ok(outcome);
        }

        // Undo never deletes or re-spawns a successor; `next_todo_id` stays.
        let todo = repositories::todos::set_completed(&mut *tx, id, false)
            .await?
            .ok_or_else(not_found)?;
        outbox::publish(&mut tx, &SystemEvent::TodoUpdated(todo.clone())).await?;
        tx.commit().await?;
        self.sync_reminder_for_todo(&todo, None).await?;
        Ok(CompletionOutcome { todo, next: None })
    }

    pub async fn update(
        &self,
        owner_id: &AccountId,
        id: i64,
        patch: TodoPatch,
    ) -> Result<(TodoItem, SyncOutcome), AppError> {
        patch.validate()?;

        let mut tx = self.pool.begin().await?;
        let current = repositories::todos::lock_owned(&mut *tx, owner_id, id)
            .await?
            .ok_or_else(not_found)?;
        let merged = patch.apply_to(&current);
        validate_merged(&merged)?;
        let anchor = (merged.recurrence_anchor != current.recurrence_anchor)
            .then_some(merged.recurrence_anchor);
        let completing = !current.completed && merged.completed;
        let remind_room = patch.remind_room_id.clone();
        let write = TodoPatch {
            // The completion itself is written by `complete_locked`.
            completed: if completing { None } else { patch.completed },
            ..patch
        };
        let updated = repositories::todos::update(&mut *tx, owner_id, id, &write, anchor)
            .await?
            .ok_or_else(not_found)?;

        if completing {
            let outcome = complete_locked(&mut tx, owner_id, updated).await?;
            tx.commit().await?;
            let sync = self.after_completion(&outcome).await?;
            return Ok((outcome.todo, sync));
        }

        outbox::publish(&mut tx, &SystemEvent::TodoUpdated(updated.clone())).await?;
        tx.commit().await?;
        let sync = self
            .sync_reminder_for_todo(&updated, remind_room.as_deref())
            .await?;
        Ok((updated, sync))
    }

    /// Post-commit reminder side effects of a completion: the successor
    /// reminds in the room of the original's latest reminder (looked up before
    /// that reminder is cancelled); the original's reminder is cancelled.
    async fn after_completion(&self, outcome: &CompletionOutcome) -> Result<SyncOutcome, AppError> {
        let original = outcome.todo.id;
        let room = match outcome.next {
            Some(_) => {
                self.reminders
                    .last_room_for_source(ReminderSource::Todo, original)
                    .await?
            }
            None => None,
        };
        let cancelled = self
            .reminders
            .cancel_due_reminder(ReminderSource::Todo, original)
            .await?;
        if let Some(next) = &outcome.next {
            self.sync_reminder_for_todo(next, room.as_deref()).await?;
        }
        Ok(cancelled)
    }
}

/// Completes the locked, not-yet-completed `current` and, when it recurs and
/// has not spawned before, inserts its successor with unchecked copies of its
/// steps. The successor is inserted directly (not through `create`) so it keeps
/// the inherited series anchor. Events: `TodoUpdated` then `TodoCreated`.
async fn complete_locked(
    tx: &mut Transaction<'_, Postgres>,
    owner_id: &AccountId,
    current: TodoItem,
) -> Result<CompletionOutcome, AppError> {
    let id = current.id;
    let mut todo = repositories::todos::set_completed(&mut **tx, id, true)
        .await?
        .ok_or_else(not_found)?;

    let mut next = None;
    if current.next_todo_id.is_none() {
        let tz = repositories::settings::get(&mut **tx, owner_id)
            .await?
            .unwrap_or_default()
            .tz();
        if let Some(new) = successor(&current, Utc::now(), tz) {
            let created = repositories::todos::create(&mut **tx, owner_id, &new).await?;
            repositories::todo_steps::copy_unchecked(&mut **tx, id, created.id, owner_id).await?;
            repositories::todos::set_next_todo_id(&mut **tx, id, created.id).await?;
            todo = repositories::todos::find_owned(&mut **tx, owner_id, id)
                .await?
                .ok_or_else(not_found)?;
            next = Some(created);
        }
    }

    outbox::publish(tx, &SystemEvent::TodoUpdated(todo.clone())).await?;
    if let Some(created) = &next {
        outbox::publish(tx, &SystemEvent::TodoCreated(created.clone())).await?;
    }
    Ok(CompletionOutcome { todo, next })
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Months, Utc};

    use crate::application::{AppError, AppServices};
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::{RecurrencePattern, ReminderSource};
    use crate::domain::tags::Tag;
    use crate::domain::todos::{NewTodo, TodoItem, TodoPatch, TodoPriority};
    use crate::infrastructure::postgres::repositories;

    async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        (pool, services, owner)
    }

    fn due() -> DateTime<Utc> {
        crate::utils::time::now_at_storage_precision() + Duration::days(2)
    }

    fn monthly(text: &str) -> NewTodo {
        NewTodo {
            room_id: "web".to_string(),
            sender_name: "alice".to_string(),
            text: text.to_string(),
            priority: TodoPriority::High,
            due_at: Some(due()),
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![Tag::parse("home").unwrap()],
            recurrence: RecurrencePattern::Monthly,
            recurrence_anchor: None,
        }
    }

    async fn create(services: &AppServices, owner: &AccountId, input: NewTodo) -> TodoItem {
        services.todos.create(owner, input).await.unwrap().0
    }

    async fn add_step(pool: &sqlx::PgPool, todo: &TodoItem, position: i32, text: &str, done: bool) {
        sqlx::query(
            "INSERT INTO todo_steps (todo_id, owner_id, position, text, done) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(todo.id)
        .bind(todo.owner_id.as_uuid())
        .bind(position)
        .bind(text)
        .bind(done)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn steps(pool: &sqlx::PgPool, todo_id: i64) -> Vec<(i32, String, bool)> {
        sqlx::query_as(
            "SELECT position, text, done FROM todo_steps WHERE todo_id = $1 ORDER BY position",
        )
        .bind(todo_id)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn todo_count(services: &AppServices, owner: &AccountId) -> i64 {
        services.todos.count(owner, None, None, None).await.unwrap()
    }

    /// Given a monthly todo with two steps (one done), When it is completed,
    /// Then one successor due a month later carries both steps unchecked.
    #[tokio::test]
    async fn completing_recurring_spawns_one_successor_with_unchecked_steps() {
        let (pool, services, owner) = setup("todo_complete_spawn").await;
        let original = create(&services, &owner, monthly("rent")).await;
        add_step(&pool, &original, 1, "transfer", true).await;
        add_step(&pool, &original, 2, "file receipt", false).await;

        let outcome = services
            .todos
            .set_completed(&owner, original.id, true)
            .await
            .unwrap();

        let next = outcome.next.expect("a successor");
        let want = original.due_at.unwrap().checked_add_months(Months::new(1));
        assert_eq!(next.due_at, want);
        assert_eq!(next.recurrence_anchor, original.recurrence_anchor);
        assert_eq!(next.tags, original.tags);
        assert!(!next.completed);
        assert!(outcome.todo.completed);
        assert_eq!(outcome.todo.next_todo_id, Some(next.id));
        assert_eq!(
            steps(&pool, next.id).await,
            vec![
                (1, "transfer".to_string(), false),
                (2, "file receipt".to_string(), false)
            ]
        );
        assert_eq!(todo_count(&services, &owner).await, 2);
    }

    /// Given a recurring todo, When two completions race on separate
    /// connections, Then the row lock lets exactly one spawn a successor.
    ///
    /// A third transaction holds the row while both completions start, so both
    /// are in flight at once before either can proceed (without `FOR UPDATE`
    /// both would read "not completed" and spawn twice).
    #[tokio::test]
    async fn concurrent_completions_spawn_once() {
        let (pool, services, owner) = setup("todo_complete_race").await;
        let original = create(&services, &owner, monthly("rent")).await;
        let mut holder = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM todos WHERE id = $1 FOR UPDATE")
            .bind(original.id)
            .execute(&mut *holder)
            .await
            .unwrap();

        let complete = |services: AppServices| {
            tokio::spawn(async move {
                services
                    .todos
                    .set_completed(&owner, original.id, true)
                    .await
            })
        };
        let (a, b) = (complete(services.clone()), complete(services.clone()));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        holder.commit().await.unwrap();

        let outcomes = [a.await.unwrap().unwrap(), b.await.unwrap().unwrap()];
        let spawned = outcomes.iter().filter(|o| o.next.is_some()).count();

        assert_eq!(spawned, 1);
        assert_eq!(todo_count(&services, &owner).await, 2);
    }

    /// Given a completed recurring todo, When it is undone and completed again,
    /// Then no second successor appears and the first one survives the undo.
    #[tokio::test]
    async fn done_undo_done_spawns_no_second_successor() {
        let (_pool, services, owner) = setup("todo_complete_undo").await;
        let original = create(&services, &owner, monthly("rent")).await;
        let todos = &services.todos;

        let first = todos
            .set_completed(&owner, original.id, true)
            .await
            .unwrap();
        let undone = todos
            .set_completed(&owner, original.id, false)
            .await
            .unwrap();
        let again = todos
            .set_completed(&owner, original.id, true)
            .await
            .unwrap();

        assert!(!undone.todo.completed);
        assert!(undone.next.is_none());
        assert_eq!(undone.todo.next_todo_id, first.todo.next_todo_id);
        assert!(again.next.is_none());
        assert!(again.todo.completed);
        assert_eq!(todo_count(&services, &owner).await, 2);
    }

    /// Given the outbox failing, When a recurring todo is completed, Then
    /// neither the completion nor the successor persists.
    #[tokio::test]
    async fn failed_publish_rolls_back_completion_and_spawn() {
        let (pool, services, owner) = setup("todo_complete_atomic").await;
        let original = create(&services, &owner, monthly("rent")).await;
        repositories::outbox::fail_every_publish(&pool).await;

        assert!(services
            .todos
            .set_completed(&owner, original.id, true)
            .await
            .is_err());

        let after = services
            .todos
            .find_owned(&owner, original.id)
            .await
            .unwrap()
            .unwrap();
        assert!(!after.completed);
        assert_eq!(after.next_todo_id, None);
        assert_eq!(todo_count(&services, &owner).await, 1);
    }

    /// Given a recurring todo, When it completes, Then the completion event
    /// precedes the successor's creation event.
    #[tokio::test]
    async fn events_are_updated_then_created() {
        let (pool, services, owner) = setup("todo_complete_events").await;
        let original = create(&services, &owner, monthly("rent")).await;

        services
            .todos
            .set_completed(&owner, original.id, true)
            .await
            .unwrap();

        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["TodoCreated", "TodoUpdated", "TodoCreated"]
        );
    }

    /// Given a web todo reminding in "!r:x", When it completes, Then the
    /// successor reminds in "!r:x" and the original's reminder is cancelled.
    #[tokio::test]
    async fn successor_reminder_uses_original_reminder_room() {
        let (_pool, services, owner) = setup("todo_complete_room").await;
        let input = NewTodo {
            remind_before_minutes: Some(60),
            remind_room_id: Some("!r:x".to_string()),
            ..monthly("rent")
        };
        let original = create(&services, &owner, input).await;

        let outcome = services
            .todos
            .set_completed(&owner, original.id, true)
            .await
            .unwrap();
        let next = outcome.next.unwrap();

        let reminders = &services.reminders;
        let successor_reminder = reminders
            .find_pending_for_source(ReminderSource::Todo, next.id)
            .await
            .unwrap()
            .expect("a pending reminder for the successor");
        assert_eq!(successor_reminder.room_id, "!r:x");
        assert!(reminders
            .find_pending_for_source(ReminderSource::Todo, original.id)
            .await
            .unwrap()
            .is_none());
    }

    /// Review focus 1: completing through a patch takes the same spawn path.
    #[tokio::test]
    async fn patch_completing_a_recurring_todo_spawns_a_successor() {
        let (_pool, services, owner) = setup("todo_complete_patch").await;
        let original = create(&services, &owner, monthly("rent")).await;
        let patch = TodoPatch {
            completed: Some(true),
            text: Some("rent (paid)".to_string()),
            ..Default::default()
        };

        let (item, _) = services
            .todos
            .update(&owner, original.id, patch)
            .await
            .unwrap();

        assert!(item.completed);
        let next_id = item.next_todo_id.expect("a successor");
        let next = services
            .todos
            .find_owned(&owner, next_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.text, "rent (paid)");
        assert_eq!(todo_count(&services, &owner).await, 2);
    }

    fn assert_due_date_validation<T: std::fmt::Debug>(result: Result<T, AppError>) {
        match result {
            Err(AppError::Validation(msg)) => assert!(msg.contains("due date"), "{msg}"),
            other => panic!("expected a validation error, got {other:?}"),
        }
    }

    /// Review focus 2: clearing the due date of a recurring todo is refused.
    #[tokio::test]
    async fn clearing_due_on_recurring_todo_is_a_validation_error() {
        let (_pool, services, owner) = setup("todo_patch_clear_due").await;
        let original = create(&services, &owner, monthly("rent")).await;
        let patch = TodoPatch {
            due_at: Some(None),
            ..Default::default()
        };

        assert_due_date_validation(services.todos.update(&owner, original.id, patch).await);

        let after = services
            .todos
            .find_owned(&owner, original.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after, original);
    }

    /// Review focus 3: turning recurrence on for a todo without a due date is refused.
    #[tokio::test]
    async fn recurrence_without_due_is_refused_on_patch() {
        let (_pool, services, owner) = setup("todo_patch_recur_no_due").await;
        let plain = NewTodo {
            due_at: None,
            recurrence: RecurrencePattern::None,
            ..monthly("someday")
        };
        let original = create(&services, &owner, plain).await;
        let patch = TodoPatch {
            recurrence: Some(RecurrencePattern::Weekly),
            ..Default::default()
        };

        assert_due_date_validation(services.todos.update(&owner, original.id, patch).await);

        let after = services
            .todos
            .find_owned(&owner, original.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.recurrence, RecurrencePattern::None);
    }
}
