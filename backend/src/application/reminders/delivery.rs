//! Reminder delivery use cases: what the `reminder.fire` job and its
//! scheduler tick need from the reminder store.

use chrono::{DateTime, Utc};

use super::ReminderService;
use crate::application::AppError;
use crate::domain::jobs::NewJob;
use crate::domain::reminders::{next_occurrence, Reminder};
use crate::events::SystemEvent;
use crate::infrastructure::postgres::repositories;

impl ReminderService {
    /// Enqueues a `reminder.fire` job for every pending reminder due by `now`.
    /// Idempotent: the dedupe key is `<id>:<trigger_at>` (R19).
    pub async fn enqueue_due(&self, now: DateTime<Utc>) -> Result<usize, AppError> {
        let due = repositories::reminders::get_due_reminders(&self.pool, now).await?;
        let mut enqueued = 0;
        for r in &due {
            let job = NewJob::reminder_fire(r.owner_id, r.id, r.trigger_at);
            if repositories::jobs::enqueue(&self.pool, &job)
                .await?
                .is_some()
            {
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }

    /// Loads a reminder by the system-generated id a job carries.
    pub async fn find_for_delivery(&self, id: i64) -> Result<Option<Reminder>, AppError> {
        Ok(repositories::reminders::find_by_id(&self.pool, id).await?)
    }

    /// The message is out: publish `ReminderTriggered` (before advancing, as
    /// the polling loop did), then reschedule a recurring reminder or finish
    /// a one-off one.
    pub async fn complete_delivery(
        &self,
        reminder: &Reminder,
        now: DateTime<Utc>,
    ) -> Result<(), AppError> {
        self.events
            .send(SystemEvent::ReminderTriggered(reminder.clone()));
        match next_occurrence(reminder.trigger_at, reminder.recurrence, now) {
            Some(next) => {
                repositories::reminders::reschedule(&self.pool, reminder.id, next).await?;
            }
            None => {
                repositories::reminders::mark_triggered(&self.pool, reminder.id).await?;
            }
        }
        Ok(())
    }

    /// Mirrors the job's attempt number into `retry_count`.
    pub async fn record_failed_attempt(
        &self,
        reminder: &Reminder,
        attempt: i32,
    ) -> Result<(), AppError> {
        repositories::reminders::record_failed_attempt(
            &self.pool,
            reminder.id,
            reminder.trigger_at,
            i64::from(attempt),
        )
        .await?;
        Ok(())
    }

    pub async fn mark_failed(&self, id: i64) -> Result<(), AppError> {
        repositories::reminders::mark_failed(&self.pool, id).await?;
        Ok(())
    }

    /// Remembers which reminder/todo a reaction on the delivered message acts on.
    pub async fn record_delivery_target(
        &self,
        reminder: &Reminder,
        event_id: &str,
    ) -> Result<(), AppError> {
        let (entity_type, entity_id) = reminder.reaction_target();
        repositories::reaction_targets::record_target(
            &self.pool,
            event_id,
            &reminder.room_id,
            entity_type,
            entity_id,
            &reminder.owner_id,
            &reminder.message,
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::domain::reminders::{NewReminder, RecurrencePattern, ReminderStatus};
    use crate::events::EventBroadcaster;

    async fn services_with_reminder(
        owner: &str,
        recurrence: RecurrencePattern,
        events: Option<&EventBroadcaster>,
    ) -> (AppServices, AccountId, Reminder, DateTime<Utc>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::new(owner);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = match events {
            Some(e) => AppServices::with_events(&pool, e),
            None => AppServices::new(&pool),
        };
        let now = crate::utils::time::now_at_storage_precision();
        let reminder = services
            .reminders
            .create(
                &owner,
                NewReminder {
                    message: "stretch".to_string(),
                    trigger_at: now + Duration::minutes(1),
                    room_id: "!room:test".to_string(),
                    owner_name: "alice".to_string(),
                    recurrence,
                    source: None,
                },
            )
            .await
            .unwrap();
        (services, owner, reminder, now)
    }

    /// Given one due reminder, When enqueue_due runs twice (a double tick),
    /// Then one job exists and the second call enqueued 0.
    #[tokio::test]
    async fn enqueue_due_is_idempotent_across_two_ticks() {
        let (services, _, _, now) =
            services_with_reminder("acct_enq_twice", RecurrencePattern::None, None).await;
        let due = now + Duration::minutes(2);

        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 1);
        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 0);
    }

    /// Given a due reminder that was snoozed after its job was enqueued, When
    /// enqueue_due runs, Then a second job exists for the new trigger.
    #[tokio::test]
    async fn a_moved_trigger_gets_its_own_job() {
        let (services, owner, reminder, now) =
            services_with_reminder("acct_enq_moved", RecurrencePattern::None, None).await;
        let due = now + Duration::minutes(10);
        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 1);

        services
            .reminders
            .update_pending(
                &owner,
                reminder.id,
                &reminder.message,
                now + Duration::minutes(5),
                RecurrencePattern::None,
            )
            .await
            .unwrap();

        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 1);
    }

    /// Given a daily reminder, When complete_delivery runs, Then
    /// ReminderTriggered is published and the reminder is pending at the next
    /// day with retry_count 0; a one-off reminder becomes triggered.
    #[tokio::test]
    async fn complete_delivery_advances_or_finishes() {
        let events = EventBroadcaster::new(16);
        let mut rx = events.subscribe();
        let (services, owner, daily, now) =
            services_with_reminder("acct_complete", RecurrencePattern::Daily, Some(&events)).await;
        services
            .reminders
            .record_failed_attempt(&daily, 2)
            .await
            .unwrap();

        services
            .reminders
            .complete_delivery(&daily, now + Duration::minutes(2))
            .await
            .unwrap();

        // Setup also published ReminderCreated; look for the trigger among them.
        let mut triggered = false;
        while let Ok(event) = rx.try_recv() {
            triggered |= matches!(event, SystemEvent::ReminderTriggered(r) if r.id == daily.id);
        }
        assert!(triggered, "ReminderTriggered must be published");
        let after = services
            .reminders
            .find_owned(&owner, daily.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, ReminderStatus::Pending);
        assert_eq!(after.trigger_at, daily.trigger_at + Duration::days(1));
        assert_eq!(after.retry_count, 0);

        let (services, owner, once, now) =
            services_with_reminder("acct_complete_once", RecurrencePattern::None, None).await;
        services
            .reminders
            .complete_delivery(&once, now + Duration::minutes(2))
            .await
            .unwrap();
        let done = services
            .reminders
            .find_owned(&owner, once.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(done.status, ReminderStatus::Triggered);
    }
}
