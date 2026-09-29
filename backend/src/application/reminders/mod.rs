//! Reminder application service.

mod delivery;

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::reminders::{
    NewReminder, RecurrencePattern, Reminder, ReminderPlan, ReminderSource, ReminderStatus,
    SyncOutcome,
};
use crate::events::{EventBroadcaster, SystemEvent};
use crate::infrastructure::postgres::repositories;

#[derive(Clone)]
pub struct ReminderService {
    pool: PgPool,
    events: EventBroadcaster,
}

impl ReminderService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
    }

    pub async fn create(
        &self,
        owner_id: &AccountId,
        input: NewReminder,
    ) -> Result<Reminder, AppError> {
        input.validate(Utc::now())?;

        let reminder = repositories::reminders::create(&self.pool, owner_id, &input).await?;
        self.broadcast_created(&reminder);

        Ok(reminder)
    }

    pub async fn find_owned(
        &self,
        owner_id: &AccountId,
        id: i64,
    ) -> Result<Option<Reminder>, AppError> {
        let reminder = repositories::reminders::find_owned(&self.pool, owner_id, id).await?;
        Ok(reminder)
    }

    pub async fn cancel(&self, owner_id: &AccountId, id: i64) -> Result<Reminder, AppError> {
        let existing = self
            .find_owned(owner_id, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Reminder not found or access denied".to_string()))?;

        let cancelled = repositories::reminders::cancel(&self.pool, owner_id, id).await?;
        if !cancelled {
            return Err(AppError::NotFound(
                "Reminder not found or no longer pending".to_string(),
            ));
        }

        self.events.send(SystemEvent::ReminderCancelled {
            id,
            room_id: existing.room_id.clone(),
            owner_id: owner_id.to_string(),
        });

        Ok(existing)
    }

    pub async fn force_cancel(
        &self,
        owner_id: &AccountId,
        id: i64,
        room_id: &str,
    ) -> Result<bool, AppError> {
        let cancelled =
            repositories::reminders::force_cancel_owned(&self.pool, owner_id, id).await?;
        if cancelled {
            self.events.send(SystemEvent::ReminderCancelled {
                id,
                room_id: room_id.to_string(),
                owner_id: owner_id.to_string(),
            });
        }
        Ok(cancelled)
    }

    pub async fn reschedule(
        &self,
        owner_id: &AccountId,
        id: i64,
        trigger_at: DateTime<Utc>,
    ) -> Result<Reminder, AppError> {
        let existing = self
            .find_owned(owner_id, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Reminder not found or access denied".to_string()))?;

        let updated = repositories::reminders::update_pending(
            &self.pool,
            owner_id,
            id,
            &existing.message,
            trigger_at,
            existing.recurrence,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Reminder not found or no longer pending".to_string()))?;

        self.broadcast_updated(&updated);
        Ok(updated)
    }

    pub async fn update_pending(
        &self,
        owner_id: &AccountId,
        id: i64,
        message: &str,
        trigger_at: DateTime<Utc>,
        recurrence: RecurrencePattern,
    ) -> Result<Reminder, AppError> {
        crate::domain::reminders::validate_message(message)?;
        crate::domain::reminders::validate_trigger_at(trigger_at, Utc::now())?;

        let updated = repositories::reminders::update_pending(
            &self.pool, owner_id, id, message, trigger_at, recurrence,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Reminder not found or no longer pending".to_string()))?;

        self.broadcast_updated(&updated);
        Ok(updated)
    }

    pub async fn mark_triggered(
        &self,
        owner_id: &AccountId,
        id: i64,
    ) -> Result<Reminder, AppError> {
        let existing = self
            .find_owned(owner_id, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Reminder not found or access denied".to_string()))?;

        repositories::reminders::mark_triggered(&self.pool, id).await?;

        let mut updated = existing;
        updated.status = ReminderStatus::Triggered;
        self.broadcast_updated(&updated);
        Ok(updated)
    }

    pub async fn clear_history(&self, owner_id: &AccountId) -> Result<(), AppError> {
        repositories::reminders::clear_user_history(&self.pool, owner_id).await?;

        self.events.send(SystemEvent::ReminderHistoryCleared {
            owner_id: owner_id.to_string(),
        });

        Ok(())
    }

    pub async fn delete_or_cancel(&self, owner_id: &AccountId, id: i64) -> Result<(), AppError> {
        let existing = self
            .find_owned(owner_id, id)
            .await?
            .ok_or_else(|| AppError::NotFound("Reminder not found or access denied".to_string()))?;

        if existing.status == ReminderStatus::Pending {
            self.cancel(owner_id, id).await?;
        } else {
            repositories::reminders::delete_history_item(&self.pool, owner_id, id).await?;
            self.events.send(SystemEvent::ReminderDeleted {
                id,
                room_id: existing.room_id,
                owner_id: owner_id.to_string(),
            });
        }

        Ok(())
    }

    pub async fn list(
        &self,
        owner_id: &AccountId,
        status: Option<&str>,
        room_id: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Reminder>, AppError> {
        let items = repositories::reminders::list_user_reminders(
            &self.pool, owner_id, status, room_id, limit, offset,
        )
        .await?;
        Ok(items)
    }

    pub async fn count(
        &self,
        owner_id: &AccountId,
        status: Option<&str>,
        room_id: Option<&str>,
    ) -> Result<i64, AppError> {
        let count =
            repositories::reminders::count_user_reminders(&self.pool, owner_id, status, room_id)
                .await?;
        Ok(count)
    }

    pub async fn sync_due_reminder(&self, plan: &ReminderPlan) -> Result<SyncOutcome, AppError> {
        let existing = repositories::reminders::find_pending_for_source(
            &self.pool,
            plan.source,
            plan.source_id,
        )
        .await?;

        let Some(trigger_at) = plan.trigger_at else {
            return self.cancel_due_reminder(plan.source, plan.source_id).await;
        };

        if trigger_at <= Utc::now() {
            self.cancel_due_reminder(plan.source, plan.source_id)
                .await?;
            return Ok(SyncOutcome::LeadTimeInPast(trigger_at));
        }

        if let Some(ref current) = existing {
            if current.trigger_at == trigger_at
                && current.room_id == plan.room_id
                && current.message == plan.message
            {
                return Ok(SyncOutcome::Scheduled(trigger_at));
            }
            self.cancel_due_reminder(plan.source, plan.source_id)
                .await?;
        }

        let new_input = NewReminder {
            message: plan.message.clone(),
            trigger_at,
            room_id: plan.room_id.clone(),
            owner_name: plan.owner_name.clone(),
            recurrence: RecurrencePattern::None,
            source: Some((plan.source, plan.source_id)),
        };

        let created =
            repositories::reminders::create(&self.pool, &plan.owner_id, &new_input).await?;
        self.broadcast_created(&created);

        Ok(SyncOutcome::Scheduled(trigger_at))
    }

    pub async fn cancel_due_reminder(
        &self,
        source: ReminderSource,
        source_id: i64,
    ) -> Result<SyncOutcome, AppError> {
        let Some(cancelled) =
            repositories::reminders::cancel_for_source(&self.pool, source, source_id).await?
        else {
            return Ok(SyncOutcome::Nothing);
        };

        self.events.send(SystemEvent::ReminderCancelled {
            id: cancelled.id,
            room_id: cancelled.room_id,
            owner_id: cancelled.owner_id.to_string(),
        });

        Ok(SyncOutcome::Cancelled)
    }

    pub async fn find_pending_for_source(
        &self,
        source: ReminderSource,
        source_id: i64,
    ) -> Result<Option<Reminder>, AppError> {
        let item =
            repositories::reminders::find_pending_for_source(&self.pool, source, source_id).await?;
        Ok(item)
    }

    fn broadcast_created(&self, r: &Reminder) {
        self.events.send(SystemEvent::ReminderCreated(r.clone()));
    }

    fn broadcast_updated(&self, r: &Reminder) {
        self.events.send(SystemEvent::ReminderUpdated(r.clone()));
    }
}
