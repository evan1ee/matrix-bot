//! Reminder delivery use cases: what the `reminder.fire` job and its
//! scheduler tick need from the reminder store.

use chrono::{DateTime, Utc};
use tracing::warn;

use super::ReminderService;
use crate::application::AppError;
use crate::domain::events::SystemEvent;
use crate::domain::jobs::{JobStatus, NewJob};
use crate::domain::reminders::{next_occurrence, Reminder};
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
            } else {
                self.fail_if_job_failed(&job, r).await?;
            }
        }
        Ok(enqueued)
    }

    /// The dedupe key blocks a re-enqueue, so a job that ended `failed`
    /// without its handler marking the reminder (timeout, database error,
    /// reaper) would leave it pending at a past trigger until the job is
    /// purged. Exhausted attempts mean failed, as the old loop had it; the
    /// guard keeps a snoozed reminder (a different key) untouched.
    async fn fail_if_job_failed(&self, job: &NewJob, r: &Reminder) -> Result<(), AppError> {
        let Some(key) = job.dedupe_key() else {
            return Ok(());
        };
        let status =
            repositories::jobs::status_by_key(&self.pool, job.job_type().as_str(), key).await?;
        if status == Some(JobStatus::Failed)
            && repositories::reminders::mark_failed_at(&self.pool, r.id, r.trigger_at).await?
        {
            warn!(
                reminder_id = r.id,
                "Reminder job failed unhandled; marked failed"
            );
        }
        Ok(())
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

    /// Fails the reminder only if it is still pending at `trigger_at`.
    pub async fn mark_failed_at(&self, id: i64, trigger_at: DateTime<Utc>) -> Result<(), AppError> {
        repositories::reminders::mark_failed_at(&self.pool, id, trigger_at).await?;
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
        let owner = AccountId::from_seed(owner);
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

    fn fire_key(r: &Reminder) -> (&'static str, String) {
        let job = NewJob::reminder_fire(r.owner_id, r.id, r.trigger_at);
        (
            job.job_type().as_str(),
            job.dedupe_key().expect("keyed").to_string(),
        )
    }

    async fn force_job_failed(services: &AppServices, r: &Reminder, pool: &sqlx::PgPool) {
        let _ = services;
        let (t, k) = fire_key(r);
        repositories::jobs::force_failed(pool, t, &k).await.unwrap();
    }

    /// Given a due reminder whose job ended failed without the handler
    /// marking it (timeout, DB error, reaper), When enqueue_due runs, Then the
    /// reminder becomes failed and no new job is created.
    #[tokio::test]
    async fn a_reminder_whose_job_failed_is_marked_failed() {
        let (services, owner, reminder, now) =
            services_with_reminder("acct_stranded", RecurrencePattern::None, None).await;
        let due = now + Duration::minutes(2);
        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 1);
        force_job_failed(&services, &reminder, &services.reminders.pool).await;

        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 0);

        let after = services
            .reminders
            .find_owned(&owner, reminder.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, ReminderStatus::Failed);
        let count = repositories::jobs::count_with_type(&services.reminders.pool, "reminder.fire")
            .await
            .unwrap();
        assert!(count >= 1);
        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 0);
    }

    /// Given a reminder snoozed after its old job failed, When enqueue_due
    /// runs, Then it is not marked failed and gets a job for the new trigger.
    #[tokio::test]
    async fn a_snoozed_reminder_is_not_failed_by_its_old_job() {
        let (services, owner, reminder, now) =
            services_with_reminder("acct_snoozed_failed", RecurrencePattern::None, None).await;
        let due = now + Duration::minutes(10);
        assert_eq!(services.reminders.enqueue_due(due).await.unwrap(), 1);
        force_job_failed(&services, &reminder, &services.reminders.pool).await;
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

        let after = services
            .reminders
            .find_owned(&owner, reminder.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(after.status, ReminderStatus::Pending);
    }
}
