//! Tag edits applied under the todo's row lock.
//!
//! `+tag -tag` is a read-modify-write of the whole tag set, so it runs inside
//! one transaction that locks the row `FOR UPDATE` first: concurrent edits
//! from the web, the AI or other chats serialise instead of overwriting each
//! other.

use super::TodoService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::tags::Tag;
use crate::domain::todos::{apply_tag_edits, TodoItem, TodoPatch};
use crate::infrastructure::postgres::repositories::{self, outbox};

impl TodoService {
    /// Adds `add` to and removes `remove` from the todo's tags (removal wins)
    /// and returns the updated todo.
    pub async fn edit_tags(
        &self,
        owner_id: &AccountId,
        id: i64,
        add: &[Tag],
        remove: &[Tag],
    ) -> Result<TodoItem, AppError> {
        let mut tx = self.pool.begin().await?;
        let current = repositories::todos::lock_owned(&mut *tx, owner_id, id)
            .await?
            .ok_or_else(|| {
                AppError::NotFound("Todo item not found or access denied".to_string())
            })?;
        let tags = apply_tag_edits(&current.tags, add, remove)?;
        let patch = TodoPatch {
            tags: Some(tags),
            ..Default::default()
        };
        // The patch touches only tags, so the recurrence anchor stays as is.
        let updated = repositories::todos::update(&mut *tx, owner_id, id, &patch, None)
            .await?
            .ok_or_else(|| {
                AppError::NotFound("Todo item not found or access denied".to_string())
            })?;
        outbox::publish(&mut tx, &SystemEvent::TodoUpdated(updated.clone())).await?;
        tx.commit().await?;
        Ok(updated)
    }
}

#[cfg(test)]
mod tests {
    use crate::application::{AppError, AppServices};
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::RecurrencePattern;
    use crate::domain::tags::Tag;
    use crate::domain::todos::{NewTodo, TodoItem, TodoPriority};
    use crate::infrastructure::postgres::repositories;

    fn tag(s: &str) -> Tag {
        Tag::parse(s).unwrap()
    }

    async fn setup(seed: &str) -> (sqlx::PgPool, AppServices, AccountId) {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        (pool, services, owner)
    }

    async fn create(services: &AppServices, owner: &AccountId, tags: Vec<Tag>) -> TodoItem {
        let input = NewTodo {
            room_id: "web".to_string(),
            sender_name: "alice".to_string(),
            text: "task".to_string(),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags,
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        };
        services.todos.create(owner, input).await.unwrap().0
    }

    #[tokio::test]
    async fn edit_tags_adds_and_removes_and_publishes_updated() {
        let (pool, services, owner) = setup("todo_edit_tags").await;
        let todo = create(&services, &owner, vec![tag("home"), tag("old")]).await;

        let updated = services
            .todos
            .edit_tags(&owner, todo.id, &[tag("work")], &[tag("old")])
            .await
            .unwrap();

        assert_eq!(updated.tags, vec![tag("home"), tag("work")]);
        let stored = services
            .todos
            .find_owned(&owner, todo.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.tags, updated.tags);
        assert_eq!(
            repositories::outbox::event_types_for(&pool, &owner)
                .await
                .unwrap(),
            vec!["TodoCreated", "TodoUpdated"]
        );
    }

    #[tokio::test]
    async fn edit_tags_on_foreign_todo_is_not_found() {
        let (pool, services, owner) = setup("todo_edit_tags_owner").await;
        let other = AccountId::from_seed("todo_edit_tags_other");
        crate::db::pool::seed_account(&pool, &other).await;
        let todo = create(&services, &owner, vec![]).await;

        let err = services
            .todos
            .edit_tags(&other, todo.id, &[tag("x")], &[])
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::NotFound(_)), "{err:?}");
    }

    /// Given a lock-holding transaction, When two edits adding different tags
    /// start on separate connections, Then both tags survive. Without
    /// `FOR UPDATE` both would read the empty set and the later write wins.
    #[tokio::test]
    async fn concurrent_edit_tags_lose_no_edit() {
        let (pool, services, owner) = setup("todo_edit_tags_race").await;
        let todo = create(&services, &owner, vec![]).await;
        let mut holder = pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM todos WHERE id = $1 FOR UPDATE")
            .bind(todo.id)
            .execute(&mut *holder)
            .await
            .unwrap();

        let edit = |name: &'static str| {
            let services = services.clone();
            tokio::spawn(async move {
                services
                    .todos
                    .edit_tags(&owner, todo.id, &[Tag::parse(name).unwrap()], &[])
                    .await
            })
        };
        let (a, b) = (edit("alpha"), edit("beta"));
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        holder.commit().await.unwrap();
        a.await.unwrap().unwrap();
        b.await.unwrap().unwrap();

        let stored = services
            .todos
            .find_owned(&owner, todo.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.tags, vec![tag("alpha"), tag("beta")]);
    }
}
