//! Todo application service.

use sqlx::PgPool;

use crate::application::reminders::ReminderService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::reminders::{ReminderPlan, ReminderSource, SyncOutcome};
use crate::domain::todos::{wanted_trigger, NewTodo, TodoItem, TodoPatch};
use crate::events::EventBroadcaster;
use crate::infrastructure::postgres::repositories;

#[derive(Clone)]
pub struct TodoService {
    pool: PgPool,
    events: EventBroadcaster,
    reminders: ReminderService,
}

impl TodoService {
    pub fn new(pool: PgPool, events: EventBroadcaster, reminders: ReminderService) -> Self {
        Self {
            pool,
            events,
            reminders,
        }
    }

    pub async fn create(
        &self,
        owner_id: &AccountId,
        input: NewTodo,
    ) -> Result<(TodoItem, SyncOutcome), AppError> {
        input.validate()?;

        let remind_room = input.remind_room_id.clone();
        let item = repositories::todos::create(&self.pool, owner_id, &input).await?;

        let sync_outcome = self
            .sync_reminder_for_todo(&item, remind_room.as_deref())
            .await?;

        self.broadcast_created(&item);

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

    pub async fn update(
        &self,
        owner_id: &AccountId,
        id: i64,
        patch: TodoPatch,
    ) -> Result<(TodoItem, SyncOutcome), AppError> {
        patch.validate()?;

        let remind_room = patch.remind_room_id.clone();
        let updated = repositories::todos::update(&self.pool, owner_id, id, &patch)
            .await?
            .ok_or_else(|| {
                AppError::NotFound("Todo item not found or access denied".to_string())
            })?;

        let sync_outcome = self
            .sync_reminder_for_todo(&updated, remind_room.as_deref())
            .await?;

        self.broadcast_updated(&updated);

        Ok((updated, sync_outcome))
    }

    pub async fn set_completed(
        &self,
        owner_id: &AccountId,
        id: i64,
        completed: bool,
    ) -> Result<TodoItem, AppError> {
        let existing = self.find_owned(owner_id, id).await?.ok_or_else(|| {
            AppError::NotFound("Todo item not found or access denied".to_string())
        })?;

        if existing.completed == completed {
            return Ok(existing);
        }

        let updated = repositories::todos::set_completed(&self.pool, id, completed)
            .await?
            .ok_or_else(|| AppError::NotFound("Todo item not found".to_string()))?;

        if completed {
            self.reminders
                .cancel_due_reminder(ReminderSource::Todo, id)
                .await?;
        } else {
            self.sync_reminder_for_todo(&updated, None).await?;
        }

        self.broadcast_updated(&updated);

        Ok(updated)
    }

    pub async fn delete(&self, owner_id: &AccountId, id: i64) -> Result<(), AppError> {
        let existing = self.find_owned(owner_id, id).await?.ok_or_else(|| {
            AppError::NotFound("Todo item not found or access denied".to_string())
        })?;

        self.reminders
            .cancel_due_reminder(ReminderSource::Todo, id)
            .await?;

        repositories::todos::delete(&self.pool, id).await?;

        self.events.send(SystemEvent::TodoDeleted {
            id,
            room_id: existing.room_id,
            owner_id: *owner_id,
        });

        Ok(())
    }

    pub async fn clear_completed(&self, owner_id: &AccountId) -> Result<u64, AppError> {
        let items = repositories::todos::list_completed_user(&self.pool, owner_id).await?;

        for item in &items {
            self.reminders
                .cancel_due_reminder(ReminderSource::Todo, item.id)
                .await?;
            self.events.send(SystemEvent::TodoDeleted {
                id: item.id,
                room_id: item.room_id.clone(),
                owner_id: *owner_id,
            });
        }

        let count = repositories::todos::clear_completed_user(&self.pool, owner_id).await?;
        Ok(count)
    }

    pub async fn clear_completed_room(&self, room_id: &str) -> Result<u64, AppError> {
        let items = repositories::todos::list_completed_room(&self.pool, room_id).await?;

        for item in &items {
            self.reminders
                .cancel_due_reminder(ReminderSource::Todo, item.id)
                .await?;
            self.events.send(SystemEvent::TodoDeleted {
                id: item.id,
                room_id: item.room_id.clone(),
                owner_id: item.owner_id,
            });
        }

        let count = repositories::todos::clear_completed_room(&self.pool, room_id).await?;
        Ok(count)
    }

    pub async fn list(
        &self,
        owner_id: &AccountId,
        room_id: Option<&str>,
        completed: Option<bool>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<TodoItem>, AppError> {
        let items = if let Some(room) = room_id {
            repositories::todos::list_room_user(
                &self.pool, room, owner_id, completed, limit, offset,
            )
            .await?
        } else {
            repositories::todos::list_all_user(&self.pool, owner_id, completed, limit, offset)
                .await?
        };

        Ok(items)
    }

    pub async fn count(
        &self,
        owner_id: &AccountId,
        room_id: Option<&str>,
        completed: Option<bool>,
    ) -> Result<i64, AppError> {
        let count = if let Some(room) = room_id {
            repositories::todos::count_room_user(&self.pool, room, owner_id, completed).await?
        } else {
            repositories::todos::count_all_user(&self.pool, owner_id, completed).await?
        };

        Ok(count)
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

    fn broadcast_created(&self, item: &TodoItem) {
        self.events.send(SystemEvent::TodoCreated(item.clone()));
    }

    fn broadcast_updated(&self, item: &TodoItem) {
        self.events.send(SystemEvent::TodoUpdated(item.clone()));
    }
}
