//! Checklist steps of a todo. Every mutation locks the todo row, so
//! concurrent step edits on one todo serialize, then publishes `TodoUpdated`.

use std::collections::{HashMap, HashSet};

use super::TodoService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::todos::steps::{validate_step_text, TodoWithSteps, MAX_STEPS};
use crate::domain::todos::{TodoError, TodoItem};
use crate::infrastructure::postgres::repositories::{self, outbox};

/// The message of a missing step; chat compares against it to say "no such
/// step" rather than "task not found".
pub const STEP_NOT_FOUND: &str = "Todo step not found";

fn todo_not_found() -> AppError {
    AppError::NotFound("Todo item not found or access denied".to_string())
}

// Mapped explicitly (not through `From<TodoError>`) so HTTP answers 404, not 400.
fn step_not_found() -> AppError {
    AppError::NotFound(STEP_NOT_FOUND.to_string())
}

impl TodoService {
    pub async fn get_with_steps(
        &self,
        owner_id: &AccountId,
        id: i64,
    ) -> Result<TodoWithSteps, AppError> {
        let todo = repositories::todos::find_owned(&self.pool, owner_id, id)
            .await?
            .ok_or_else(todo_not_found)?;
        let steps = repositories::todo_steps::list(&self.pool, id).await?;
        Ok(TodoWithSteps { todo, steps })
    }

    pub async fn add_step(
        &self,
        owner_id: &AccountId,
        todo_id: i64,
        text: &str,
    ) -> Result<TodoWithSteps, AppError> {
        let text = validate_step_text(text)?;
        let mut tx = self.pool.begin().await?;
        let todo = lock(&mut tx, owner_id, todo_id).await?;
        if repositories::todo_steps::count(&mut *tx, todo_id).await? >= MAX_STEPS as i64 {
            return Err(TodoError::TooManySteps.into());
        }
        repositories::todo_steps::insert_last(&mut *tx, todo_id, owner_id, &text).await?;
        finish(tx, todo).await
    }

    pub async fn update_step(
        &self,
        owner_id: &AccountId,
        todo_id: i64,
        step_id: i64,
        text: Option<&str>,
        done: Option<bool>,
    ) -> Result<TodoWithSteps, AppError> {
        let text = text.map(validate_step_text).transpose()?;
        let mut tx = self.pool.begin().await?;
        let todo = lock(&mut tx, owner_id, todo_id).await?;
        repositories::todo_steps::update(&mut *tx, todo_id, step_id, text.as_deref(), done)
            .await?
            .ok_or_else(step_not_found)?;
        finish(tx, todo).await
    }

    pub async fn remove_step(
        &self,
        owner_id: &AccountId,
        todo_id: i64,
        step_id: i64,
    ) -> Result<TodoWithSteps, AppError> {
        let mut tx = self.pool.begin().await?;
        let todo = lock(&mut tx, owner_id, todo_id).await?;
        if !repositories::todo_steps::delete(&mut *tx, todo_id, step_id).await? {
            return Err(step_not_found());
        }
        repositories::todo_steps::renumber(&mut tx, todo_id).await?;
        finish(tx, todo).await
    }

    /// `ids` must list every step of the todo exactly once.
    pub async fn reorder_steps(
        &self,
        owner_id: &AccountId,
        todo_id: i64,
        ids: &[i64],
    ) -> Result<TodoWithSteps, AppError> {
        let mut tx = self.pool.begin().await?;
        let todo = lock(&mut tx, owner_id, todo_id).await?;
        let current: HashSet<i64> = repositories::todo_steps::list(&mut *tx, todo_id)
            .await?
            .into_iter()
            .map(|s| s.id)
            .collect();
        let wanted: HashSet<i64> = ids.iter().copied().collect();
        if wanted.len() != ids.len() || wanted != current {
            return Err(TodoError::StepOrderMismatch.into());
        }
        repositories::todo_steps::set_order(&mut tx, todo_id, ids).await?;
        finish(tx, todo).await
    }

    /// `(done, total)` per todo id, for list views. Todos without steps are absent.
    pub async fn step_progress(
        &self,
        owner_id: &AccountId,
        ids: &[i64],
    ) -> Result<HashMap<i64, (i64, i64)>, AppError> {
        let rows = repositories::todo_steps::progress_for(&self.pool, owner_id, ids).await?;
        Ok(rows.into_iter().map(|(id, d, t)| (id, (d, t))).collect())
    }
}

async fn lock(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner_id: &AccountId,
    todo_id: i64,
) -> Result<TodoItem, AppError> {
    repositories::todos::lock_owned(&mut **tx, owner_id, todo_id)
        .await?
        .ok_or_else(todo_not_found)
}

async fn finish(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    todo: TodoItem,
) -> Result<TodoWithSteps, AppError> {
    let steps = repositories::todo_steps::list(&mut *tx, todo.id).await?;
    outbox::publish(&mut tx, &SystemEvent::TodoUpdated(todo.clone())).await?;
    tx.commit().await?;
    Ok(TodoWithSteps { todo, steps })
}

#[cfg(test)]
mod tests {
    use crate::application::{AppError, AppServices};
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::RecurrencePattern;
    use crate::domain::todos::{NewTodo, TodoPriority};
    use crate::infrastructure::postgres::repositories;

    fn new_todo(text: &str) -> NewTodo {
        NewTodo {
            room_id: "web".to_string(),
            sender_name: "t".to_string(),
            text: text.to_string(),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        }
    }

    async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId, i64) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let (todo, _) = services
            .todos
            .create(&owner, new_todo("todo"))
            .await
            .unwrap();
        (pool, services, owner, todo.id)
    }

    /// Given 50 steps, When a 51st is added, Then it is refused as a validation error.
    #[tokio::test]
    async fn add_step_refuses_the_51st() {
        let (_pool, s, owner, id) = setup("steps_svc_51").await;
        for i in 0..50 {
            s.todos
                .add_step(&owner, id, &format!("s{i}"))
                .await
                .unwrap();
        }
        let err = s.todos.add_step(&owner, id, "one more").await.unwrap_err();
        assert!(matches!(&err, AppError::Validation(m) if m.contains("50 steps")));
    }

    /// Given another owner's todo, When steps are touched, Then NotFound.
    #[tokio::test]
    async fn step_of_another_owners_todo_is_not_found() {
        let (pool, s, _owner, id) = setup("steps_svc_owner").await;
        let other = AccountId::from_seed("steps_svc_owner_other");
        crate::db::pool::seed_account(&pool, &other).await;
        let err = s.todos.add_step(&other, id, "x").await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
        assert!(matches!(
            s.todos.get_with_steps(&other, id).await,
            Err(AppError::NotFound(_))
        ));
    }

    /// Given a step of todo A, When todo B's endpoint names it, Then NotFound.
    #[tokio::test]
    async fn step_id_from_a_different_todo_is_not_found() {
        let (_pool, s, owner, a) = setup("steps_svc_cross").await;
        let (b, _) = s.todos.create(&owner, new_todo("b")).await.unwrap();
        let step = s.todos.add_step(&owner, a, "x").await.unwrap().steps[0].id;
        assert!(matches!(
            s.todos
                .update_step(&owner, b.id, step, None, Some(true))
                .await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            s.todos.remove_step(&owner, b.id, step).await,
            Err(AppError::NotFound(_))
        ));
    }

    /// Given two steps, When reorder gets a partial, duplicate or foreign id set, Then it is refused.
    #[tokio::test]
    async fn reorder_requires_exact_id_set() {
        let (_pool, s, owner, id) = setup("steps_svc_reorder").await;
        s.todos.add_step(&owner, id, "a").await.unwrap();
        let both = s.todos.add_step(&owner, id, "b").await.unwrap().steps;
        let (a, b) = (both[0].id, both[1].id);
        for bad in [vec![a], vec![a, a], vec![a, b, b + 999]] {
            let err = s.todos.reorder_steps(&owner, id, &bad).await.unwrap_err();
            assert!(matches!(err, AppError::Validation(_)), "{bad:?}");
        }
        let ok = s.todos.reorder_steps(&owner, id, &[b, a]).await.unwrap();
        assert_eq!(ok.step_at(1).unwrap().text, "b");
        assert!(ok.step_at(0).is_none() && ok.step_at(3).is_none());
    }

    /// Given five step mutations, Then five TodoUpdated events are published.
    #[tokio::test]
    async fn each_step_mutation_publishes_todo_updated() {
        let (pool, s, owner, id) = setup("steps_svc_events").await;
        let a = s.todos.add_step(&owner, id, "a").await.unwrap().steps[0].id;
        let b = s.todos.add_step(&owner, id, "b").await.unwrap().steps[1].id;
        s.todos
            .update_step(&owner, id, a, Some("a2"), Some(true))
            .await
            .unwrap();
        s.todos.reorder_steps(&owner, id, &[b, a]).await.unwrap();
        let after = s.todos.remove_step(&owner, id, b).await.unwrap();
        assert_eq!(after.steps.len(), 1);
        assert_eq!(after.steps[0].position, 1);
        let events = repositories::outbox::event_types_for(&pool, &owner)
            .await
            .unwrap();
        assert_eq!(
            events.iter().filter(|e| *e == "TodoUpdated").count(),
            5,
            "{events:?}"
        );
        let progress = s.todos.step_progress(&owner, &[id]).await.unwrap();
        assert_eq!(progress.get(&id), Some(&(1, 1)));
    }
}
