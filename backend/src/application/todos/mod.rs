//! Todo application service.

mod completion;
mod steps;
mod tags;

pub use completion::CompletionOutcome;

use sqlx::PgPool;

use crate::application::reminders::ReminderService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::reminders::RecurrencePattern;
use crate::domain::reminders::{ReminderPlan, ReminderSource, SyncOutcome};
use crate::domain::tags::Tag;
use crate::domain::todos::{wanted_trigger, NewTodo, TodoError, TodoItem};
use crate::infrastructure::postgres::repositories::{self, outbox};

impl From<TodoError> for AppError {
    fn from(err: TodoError) -> Self {
        AppError::Validation(err.to_string())
    }
}

#[derive(Clone)]
pub struct TodoService {
    pool: PgPool,
    reminders: ReminderService,
}

impl TodoService {
    pub fn new(pool: PgPool, reminders: ReminderService) -> Self {
        Self { pool, reminders }
    }

    pub async fn create(
        &self,
        owner_id: &AccountId,
        input: NewTodo,
    ) -> Result<(TodoItem, SyncOutcome), AppError> {
        input.validate()?;
        // A recurring series is anchored to the due date it starts from.
        let input = NewTodo {
            recurrence_anchor: (input.recurrence != RecurrencePattern::None)
                .then_some(input.due_at)
                .flatten(),
            ..input
        };

        let remind_room = input.remind_room_id.clone();
        let mut tx = self.pool.begin().await?;
        let item = repositories::todos::create(&mut *tx, owner_id, &input).await?;
        outbox::publish(&mut tx, &SystemEvent::TodoCreated(item.clone())).await?;
        tx.commit().await?;

        let sync_outcome = self
            .sync_reminder_for_todo(&item, remind_room.as_deref())
            .await?;

        Ok((item, sync_outcome))
    }

    pub async fn find_owned(
        &self,
        owner_id: &AccountId,
        id: i64,
    ) -> Result<Option<TodoItem>, AppError> {
        let item = repositories::todos::find_owned(&self.pool, owner_id, id).await?;
        Ok(item)
    }

    pub async fn delete(&self, owner_id: &AccountId, id: i64) -> Result<(), AppError> {
        let existing = self.find_owned(owner_id, id).await?.ok_or_else(|| {
            AppError::NotFound("Todo item not found or access denied".to_string())
        })?;

        self.reminders
            .cancel_due_reminder(ReminderSource::Todo, id)
            .await?;

        let mut tx = self.pool.begin().await?;
        repositories::todos::delete(&mut *tx, id).await?;
        outbox::publish(
            &mut tx,
            &SystemEvent::TodoDeleted {
                id,
                room_id: existing.room_id,
                owner_id: *owner_id,
            },
        )
        .await?;
        tx.commit().await?;

        Ok(())
    }

    /// Cancels each completed todo's reminder first (its own transactions, as
    /// before), then lists, deletes and publishes one `TodoDeleted` per item in
    /// a single transaction (C4).
    pub async fn clear_completed(&self, owner_id: &AccountId) -> Result<u64, AppError> {
        let items = repositories::todos::list_completed_user(&self.pool, owner_id).await?;
        self.cancel_reminders(&items).await?;

        let mut tx = self.pool.begin().await?;
        let items = repositories::todos::list_completed_user(&mut *tx, owner_id).await?;
        let count = repositories::todos::clear_completed_user(&mut *tx, owner_id).await?;
        Self::publish_deleted(&mut tx, &items).await?;
        tx.commit().await?;
        Ok(count)
    }

    pub async fn clear_completed_room(&self, room_id: &str) -> Result<u64, AppError> {
        let items = repositories::todos::list_completed_room(&self.pool, room_id).await?;
        self.cancel_reminders(&items).await?;

        let mut tx = self.pool.begin().await?;
        let items = repositories::todos::list_completed_room(&mut *tx, room_id).await?;
        let count = repositories::todos::clear_completed_room(&mut *tx, room_id).await?;
        Self::publish_deleted(&mut tx, &items).await?;
        tx.commit().await?;
        Ok(count)
    }

    async fn cancel_reminders(&self, items: &[TodoItem]) -> Result<(), AppError> {
        for item in items {
            self.reminders
                .cancel_due_reminder(ReminderSource::Todo, item.id)
                .await?;
        }
        Ok(())
    }

    async fn publish_deleted(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        items: &[TodoItem],
    ) -> Result<(), AppError> {
        for item in items {
            let event = SystemEvent::TodoDeleted {
                id: item.id,
                room_id: item.room_id.clone(),
                owner_id: item.owner_id,
            };
            outbox::publish(tx, &event).await?;
        }
        Ok(())
    }

    pub async fn list(
        &self,
        owner_id: &AccountId,
        room_id: Option<&str>,
        completed: Option<bool>,
        tag: Option<&Tag>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TodoItem>, AppError> {
        let tag = tag.map(Tag::as_str);
        let items = if let Some(room) = room_id {
            repositories::todos::list_room_user(
                &self.pool, room, owner_id, completed, tag, limit, offset,
            )
            .await?
        } else {
            repositories::todos::list_all_user(&self.pool, owner_id, completed, tag, limit, offset)
                .await?
        };

        Ok(items)
    }

    pub async fn count(
        &self,
        owner_id: &AccountId,
        room_id: Option<&str>,
        completed: Option<bool>,
        tag: Option<&Tag>,
    ) -> Result<i64, AppError> {
        let tag = tag.map(Tag::as_str);
        let count = if let Some(room) = room_id {
            repositories::todos::count_room_user(&self.pool, room, owner_id, completed, tag).await?
        } else {
            repositories::todos::count_all_user(&self.pool, owner_id, completed, tag).await?
        };

        Ok(count)
    }

    /// The owner's tags over active todos with their todo counts. A stored
    /// value that no longer parses predates validation and is skipped.
    pub async fn list_tags(&self, owner_id: &AccountId) -> Result<Vec<(Tag, i64)>, AppError> {
        let rows = repositories::todos::list_tags(&self.pool, owner_id).await?;
        Ok(rows
            .into_iter()
            .filter_map(|(raw, count)| Tag::parse(&raw).ok().map(|tag| (tag, count)))
            .collect())
    }

    pub async fn list_room(
        &self,
        room_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TodoItem>, AppError> {
        let items = repositories::todos::list_room(&self.pool, room_id, limit, offset).await?;
        Ok(items)
    }

    pub async fn count_room_active(&self, room_id: &str) -> Result<i64, AppError> {
        let count = repositories::todos::count_room_active(&self.pool, room_id).await?;
        Ok(count)
    }

    async fn sync_reminder_for_todo(
        &self,
        todo: &TodoItem,
        override_room: Option<&str>,
    ) -> Result<SyncOutcome, AppError> {
        if wanted_trigger(todo).is_none() {
            return self
                .reminders
                .cancel_due_reminder(ReminderSource::Todo, todo.id)
                .await;
        }

        let current_room = self
            .reminders
            .find_pending_for_source(ReminderSource::Todo, todo.id)
            .await?
            .map(|r| r.room_id);

        let room_id = match override_room {
            Some(r) if !r.trim().is_empty() => r.to_string(),
            _ => match current_room {
                Some(r) => r,
                None => {
                    if todo.room_id == "web" || todo.room_id.trim().is_empty() {
                        return Err(AppError::Validation(
                            "remind_room_id is required for a reminder on a task that has no Matrix room"
                                .to_string(),
                        ));
                    }
                    todo.room_id.clone()
                }
            },
        };

        let plan = ReminderPlan {
            source: ReminderSource::Todo,
            source_id: todo.id,
            owner_id: todo.owner_id,
            owner_name: todo.sender_name.clone(),
            room_id,
            message: format!("Task #{} is due: {}", todo.id, todo.text),
            trigger_at: wanted_trigger(todo),
        };

        self.reminders.sync_due_reminder(&plan).await
    }
}

#[cfg(test)]
mod tests {
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::RecurrencePattern;
    use crate::domain::tags::Tag;
    use crate::domain::todos::{NewTodo, TodoPatch, TodoPriority};
    use crate::infrastructure::postgres::repositories;

    fn tag(raw: &str) -> Tag {
        Tag::parse(raw).unwrap()
    }

    fn new_todo(text: &str) -> NewTodo {
        NewTodo {
            room_id: "web".to_string(),
            sender_name: "alice".to_string(),
            text: text.to_string(),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: crate::domain::reminders::RecurrencePattern::None,
            recurrence_anchor: None,
        }
    }

    async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        (pool, services, owner)
    }

    /// Given an owner, When a todo is created, Then TodoCreated is in the outbox.
    #[tokio::test]
    async fn create_writes_todo_created_to_the_outbox() {
        let (pool, services, owner) = setup("todo_outbox_create").await;

        services
            .todos
            .create(&owner, new_todo("buy milk"))
            .await
            .unwrap();

        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["TodoCreated"]
        );
    }

    /// Given the outbox insert failing, When create runs, Then the todo row
    /// the same transaction inserted is rolled back and the call errors.
    #[tokio::test]
    async fn a_failed_create_leaves_no_row_and_no_event() {
        let (pool, services, owner) = setup("todo_outbox_atomic").await;
        repositories::outbox::fail_every_publish(&pool).await;

        assert!(services
            .todos
            .create(&owner, new_todo("buy milk"))
            .await
            .is_err());

        assert_eq!(
            services
                .todos
                .count(&owner, None, None, None)
                .await
                .unwrap(),
            0
        );
        assert!(repositories::outbox::event_types_for(&pool, &owner)
            .await
            .unwrap()
            .is_empty());
    }

    /// Given completed todos, When clear_completed runs, Then one TodoDeleted
    /// per item is written together with the delete (C4).
    #[tokio::test]
    async fn clear_completed_publishes_one_deleted_per_item() {
        let (pool, services, owner) = setup("todo_outbox_clear").await;
        for text in ["a", "b"] {
            let (item, _) = services.todos.create(&owner, new_todo(text)).await.unwrap();
            services
                .todos
                .set_completed(&owner, item.id, true)
                .await
                .unwrap();
        }

        assert_eq!(services.todos.clear_completed(&owner).await.unwrap(), 2);

        let types = repositories::outbox::event_types_for(&pool, &owner)
            .await
            .unwrap();
        assert_eq!(types.iter().filter(|t| *t == "TodoDeleted").count(), 2);
    }

    /// Given recurrence and a due date, When the todo is created, Then the series is anchored to it.
    #[tokio::test]
    async fn create_anchors_a_recurring_todo_to_its_due_date() {
        let (_pool, services, owner) = setup("todo_anchor_create").await;
        let due = crate::utils::time::now_at_storage_precision();
        let input = NewTodo {
            due_at: Some(due),
            recurrence: RecurrencePattern::Monthly,
            ..new_todo("rent")
        };
        let (item, _) = services.todos.create(&owner, input).await.unwrap();
        assert_eq!(item.recurrence_anchor, Some(due));

        let (plain, _) = services
            .todos
            .create(&owner, new_todo("plain"))
            .await
            .unwrap();
        assert_eq!(plain.recurrence_anchor, None);
    }

    /// Given a todo, When recurrence is turned on, the due date moved, or recurrence
    /// cleared, Then the anchor follows the rule (and ignores unrelated edits).
    #[tokio::test]
    async fn update_maintains_the_recurrence_anchor() {
        let (_pool, services, owner) = setup("todo_anchor_update").await;
        let due = crate::utils::time::now_at_storage_precision();
        let later = due + chrono::Duration::days(3);
        let input = NewTodo {
            due_at: Some(due),
            ..new_todo("rent")
        };
        let (item, _) = services.todos.create(&owner, input).await.unwrap();
        assert_eq!(item.recurrence_anchor, None);

        let on = TodoPatch {
            recurrence: Some(RecurrencePattern::Weekly),
            ..Default::default()
        };
        let (item, _) = services.todos.update(&owner, item.id, on).await.unwrap();
        assert_eq!(item.recurrence_anchor, Some(due));

        let rename = TodoPatch {
            text: Some("rent!".to_string()),
            ..Default::default()
        };
        let (item, _) = services
            .todos
            .update(&owner, item.id, rename)
            .await
            .unwrap();
        assert_eq!(item.recurrence_anchor, Some(due));

        let moved = TodoPatch {
            due_at: Some(Some(later)),
            ..Default::default()
        };
        let (item, _) = services.todos.update(&owner, item.id, moved).await.unwrap();
        assert_eq!(item.recurrence_anchor, Some(later));

        let off = TodoPatch {
            recurrence: Some(RecurrencePattern::None),
            ..Default::default()
        };
        let (item, _) = services.todos.update(&owner, item.id, off).await.unwrap();
        assert_eq!(item.recurrence_anchor, None);
    }

    /// Given a tagged todo, When a patch replaces the tag set, Then the new
    /// set is stored and TodoUpdated is published.
    #[tokio::test]
    async fn update_replaces_tag_set_and_publishes_updated() {
        let (pool, services, owner) = setup("todo_tags_replace").await;
        let input = NewTodo {
            tags: vec![tag("home")],
            ..new_todo("sweep")
        };
        let (item, _) = services.todos.create(&owner, input).await.unwrap();

        let patch = TodoPatch {
            tags: Some(vec![tag("work"), tag("urgent")]),
            ..Default::default()
        };
        let (updated, _) = services.todos.update(&owner, item.id, patch).await.unwrap();

        assert_eq!(updated.tags, vec![tag("urgent"), tag("work")]);
        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["TodoCreated", "TodoUpdated"]
        );
    }

    /// Given two owners with tagged todos, When each lists tags, Then each
    /// sees only their own, counted over active todos.
    #[tokio::test]
    async fn list_tags_is_owner_scoped() {
        let (pool, services, alice) = setup("todo_tags_alice").await;
        let bob = AccountId::from_seed("todo_tags_bob");
        crate::db::pool::seed_account(&pool, &bob).await;
        for (owner, text, tags) in [
            (&alice, "a1", vec![tag("home"), tag("work")]),
            (&alice, "a2", vec![tag("home")]),
            (&bob, "b1", vec![tag("secret")]),
        ] {
            let input = NewTodo {
                tags,
                ..new_todo(text)
            };
            services.todos.create(owner, input).await.unwrap();
        }

        assert_eq!(
            services.todos.list_tags(&alice).await.unwrap(),
            vec![(tag("home"), 2), (tag("work"), 1)]
        );
        assert_eq!(
            services.todos.list_tags(&bob).await.unwrap(),
            vec![(tag("secret"), 1)]
        );
    }

    /// Given tagged todos, When listing and counting with a tag filter, Then
    /// only matching todos are returned.
    #[tokio::test]
    async fn list_and_count_filter_by_tag() {
        let (_pool, services, owner) = setup("todo_tags_filter").await;
        for (text, tags) in [("x", vec![tag("home")]), ("y", vec![tag("work")])] {
            let input = NewTodo {
                tags,
                ..new_todo(text)
            };
            services.todos.create(&owner, input).await.unwrap();
        }
        let home = tag("home");

        let items = services
            .todos
            .list(&owner, None, None, Some(&home), 50, 0)
            .await
            .unwrap();
        let count = services
            .todos
            .count(&owner, None, None, Some(&home))
            .await
            .unwrap();

        assert_eq!(
            items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            ["x"]
        );
        assert_eq!(count, 1);
    }
}
