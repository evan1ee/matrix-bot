//! `reminder.fire`: delivers one due reminder to the conversation it was set in.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, error, info, warn};

use super::{deliverable, message, parse_payload, Outbound, SendFailure, CHANNEL_RETRY};
use crate::application::AppServices;
use crate::domain::jobs::{JobType, ReminderFirePayload};
use crate::domain::reminders::Reminder;
use crate::interface::channels::message::Target;
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::utils::{html_escape, strip_html};

pub struct ReminderFireHandler {
    services: AppServices,
    outbound: Arc<dyn Outbound>,
}

impl ReminderFireHandler {
    pub fn new(services: AppServices, outbound: Arc<dyn Outbound>) -> Self {
        Self { services, outbound }
    }

    async fn failed_attempt(
        &self,
        reminder: &Reminder,
        ctx: &JobContext,
        reason: String,
    ) -> Result<(), JobError> {
        if ctx.is_last_attempt() {
            error!(
                reminder_id = reminder.id,
                attempts = ctx.attempt,
                "Reminder exhausted its delivery attempts; handling exhaustion"
            );
            self.services
                .reminders
                .mark_failed_or_advance_recurring(reminder, reminder.trigger_at, Utc::now())
                .await?;
            return Err(JobError::Permanent(anyhow::anyhow!(reason)));
        }
        if let Err(err) = self
            .services
            .reminders
            .record_failed_attempt(reminder, ctx.attempt)
            .await
        {
            warn!(reminder_id = reminder.id, error = %err, "Could not mirror retry_count");
        }
        warn!(
            reminder_id = reminder.id,
            attempt = ctx.attempt,
            max = ctx.max_attempts,
            %reason,
            "Reminder delivery failed; will retry"
        );
        Err(JobError::Retryable(anyhow::anyhow!(reason)))
    }
}

#[async_trait]
impl JobHandler for ReminderFireHandler {
    fn job_type(&self) -> JobType {
        JobType::ReminderFire
    }

    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError> {
        let p: ReminderFirePayload = parse_payload(payload)?;
        match self.fire(ctx, &p).await {
            // Defence in depth: the final attempt must not leave the reminder
            // pending behind a failed job (e.g. a database error above).
            Err(JobError::Retryable(err)) if ctx.is_last_attempt() => {
                error!(reminder_id = p.reminder_id, error = %err, "Last attempt failed; handling reminder exhaustion");
                if let Ok(Some(reminder)) = self
                    .services
                    .reminders
                    .find_for_delivery(p.reminder_id)
                    .await
                {
                    if let Err(advance_err) = self
                        .services
                        .reminders
                        .mark_failed_or_advance_recurring(&reminder, p.trigger_at, Utc::now())
                        .await
                    {
                        warn!(reminder_id = p.reminder_id, error = %advance_err, "Could not advance or mark the reminder failed");
                    }
                } else if let Err(mark_err) = self
                    .services
                    .reminders
                    .mark_failed_at(p.reminder_id, p.trigger_at)
                    .await
                {
                    warn!(reminder_id = p.reminder_id, error = %mark_err, "Could not mark the reminder failed");
                }
                Err(JobError::Permanent(err))
            }
            other => other,
        }
    }
}

impl ReminderFireHandler {
    async fn fire(&self, ctx: &JobContext, p: &ReminderFirePayload) -> Result<(), JobError> {
        let Some(reminder) = self
            .services
            .reminders
            .find_for_delivery(p.reminder_id)
            .await?
        else {
            debug!(
                reminder_id = p.reminder_id,
                "Reminder gone; nothing to fire"
            );
            return Ok(());
        };
        if !reminder.is_pending_at(p.trigger_at) {
            debug!(
                reminder_id = reminder.id,
                "Reminder changed since it was enqueued; skipping"
            );
            return Ok(());
        }

        let tz = self
            .services
            .settings
            .get_or_default(&reminder.owner_id)
            .await
            .tz();
        let html = format_reminder_alert(
            &reminder.message,
            &reminder.trigger_at,
            &tz,
            reminder.recurrence.as_str(),
        );
        let plain = strip_html(&html);
        // A room that is no address (or the web UI) has nothing to deliver to;
        // the reminder counts as delivered, as it always did.
        let Some((address, channel)) = deliverable(&reminder.room_id) else {
            self.services
                .reminders
                .complete_delivery(&reminder, Utc::now())
                .await?;
            return Ok(());
        };
        // Checked per channel and before sending, so a reminder for a platform
        // that is down waits instead of burning an attempt.
        if !self.outbound.is_connected(channel) {
            return Err(JobError::Unavailable(CHANNEL_RETRY));
        }
        if let Err(reason) = self
            .outbound
            .authorize_delivery(&reminder.owner_id, &address)
            .await
        {
            error!(
                reminder_id = reminder.id,
                %reason,
                "Reminder delivery unauthorized; marking failed"
            );
            self.services.reminders.mark_failed(reminder.id).await?;
            return Err(JobError::Permanent(anyhow::anyhow!(reason)));
        }
        let sent = self
            .outbound
            .send(message(Target::Conversation(address), &plain, &html))
            .await;
        match sent {
            Ok(sent) => {
                info!(reminder_id = reminder.id, room = %reminder.room_id, "Delivered reminder");
                if let Some(message) = sent.message {
                    if let Err(err) = self
                        .services
                        .reminders
                        .record_delivery_target(&reminder, &message.0)
                        .await
                    {
                        warn!(reminder_id = reminder.id, error = %err, "Could not record the reaction target");
                    }
                }
                self.services
                    .reminders
                    .complete_delivery(&reminder, Utc::now())
                    .await?;
                Ok(())
            }
            Err(err) => match SendFailure::from(err) {
                SendFailure::Defer => Err(JobError::Unavailable(CHANNEL_RETRY)),
                SendFailure::Retry(reason) => self.failed_attempt(&reminder, ctx, reason).await,
                SendFailure::Permanent(reason) => {
                    error!(reminder_id = reminder.id, %reason, "Reminder delivery rejected; marking failed");
                    self.services.reminders.mark_failed(reminder.id).await?;
                    Err(JobError::Permanent(anyhow::anyhow!(reason)))
                }
            },
        }
    }
}

pub fn format_reminder_alert(
    message: &str,
    trigger_at: &DateTime<Utc>,
    tz: &chrono_tz::Tz,
    recurrence: &str,
) -> String {
    let local_time = trigger_at
        .with_timezone(tz)
        .format("%Y-%m-%d %H:%M")
        .to_string();
    let mut html = format!(
        "⏰ <b>提醒时间到了</b><br/>• <b>事项:</b> {}<br/>• <b>设定时间:</b> {}",
        html_escape(message),
        local_time
    );
    let rec_label = match recurrence {
        "daily" => Some("每天重复"),
        "weekly" => Some("每周重复"),
        "monthly" => Some("每月重复"),
        "yearly" => Some("每年重复"),
        _ => None,
    };
    if let Some(rec) = rec_label {
        html.push_str(&format!("<br/>• <b>周期:</b> {}", rec));
    }
    html.push_str("<br/><font color=\"#888888\" size=\"-1\">React: 👍️ Done · 👎️ Delete · 😝 +1 Day · 🎉 Celebrate</font>");
    html
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::channels::ConversationAddress;
    use crate::domain::jobs::JobId;
    use crate::domain::reminders::{
        NewReminder, RecurrencePattern, ReminderSource, ReminderStatus,
    };
    use crate::infrastructure::postgres::repositories;
    use crate::interface::channels::adapter::ChannelError;
    use crate::interface::channels::matrix::reaction_targets::get_target;
    use crate::interface::channels::message::Target;
    use crate::interface::jobs::fake::FakeOutbound;

    fn utc(y: i32, m: u32, d: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, 0, 0).unwrap()
    }

    #[test]
    fn test_format_reminder_alert_non_recurring() {
        let t = utc(2026, 9, 17, 12);
        let tz = chrono_tz::Asia::Shanghai;
        let html = format_reminder_alert("去跑步", &t, &tz, "none");
        assert!(html.contains("⏰ <b>提醒时间到了</b>"));
        assert!(html.contains("• <b>事项:</b> 去跑步"));
        assert!(html.contains("• <b>设定时间:</b> 2026-09-17 20:00"));
        assert!(!html.contains("周期"));
    }

    #[test]
    fn test_format_reminder_alert_recurring() {
        let t = utc(2026, 9, 17, 12);
        let tz = chrono_tz::Asia::Shanghai;
        let html = format_reminder_alert("去跑步", &t, &tz, "daily");
        assert!(html.contains("• <b>周期:</b> 每天重复"));
    }

    struct Fixture {
        pool: sqlx::PgPool,
        services: AppServices,
        owner: AccountId,
        reminder: Reminder,
    }

    async fn fixture(owner: &str, recurrence: RecurrencePattern) -> Fixture {
        fixture_from(owner, recurrence, None).await
    }

    async fn fixture_from(
        owner: &str,
        recurrence: RecurrencePattern,
        source: Option<(ReminderSource, i64)>,
    ) -> Fixture {
        fixture_in(owner, recurrence, source, "!room:test").await
    }

    async fn fixture_in(
        owner: &str,
        recurrence: RecurrencePattern,
        source: Option<(ReminderSource, i64)>,
        room: &str,
    ) -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(owner);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        let reminder = services
            .reminders
            .create(
                &owner,
                NewReminder {
                    message: "stretch <now>".to_string(),
                    trigger_at: crate::utils::time::now_at_storage_precision()
                        + Duration::minutes(1),
                    room_id: room.to_string(),
                    owner_name: "alice".to_string(),
                    recurrence,
                    source,
                },
            )
            .await
            .unwrap();
        Fixture {
            pool,
            services,
            owner,
            reminder,
        }
    }

    fn ctx(owner: &AccountId, attempt: i32) -> JobContext {
        JobContext {
            job_id: JobId(1),
            account_id: Some(*owner),
            attempt,
            max_attempts: 5,
            cancel: CancellationToken::new(),
        }
    }

    fn payload(r: &Reminder, trigger_at: DateTime<Utc>) -> serde_json::Value {
        serde_json::json!({ "reminder_id": r.id, "trigger_at": trigger_at })
    }

    async fn current(f: &Fixture) -> Reminder {
        f.services
            .reminders
            .find_owned(&f.owner, f.reminder.id)
            .await
            .unwrap()
            .unwrap()
    }

    #[tokio::test]
    async fn delivers_formats_in_the_owners_zone_records_the_target_and_advances() {
        let f = fixture("acct_fire_ok", RecurrencePattern::None).await;
        let fake = FakeOutbound::connected();
        let handler = ReminderFireHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(out.is_ok(), "{out:?}");
        let sends = fake.room_sends();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, "!room:test");
        assert!(sends[0].2.contains("提醒时间到了"));
        assert!(sends[0].2.contains("stretch &lt;now&gt;"), "escaped");
        let target = get_target(&f.pool, "$evt").await.unwrap().unwrap();
        assert_eq!(target.entity_type, "reminder");
        assert_eq!(target.entity_id, f.reminder.id);
        assert_eq!(current(&f).await.status, ReminderStatus::Triggered);
        assert!(
            repositories::outbox::event_types_for(&f.pool, &f.owner)
                .await
                .unwrap()
                .contains(&"ReminderTriggered".to_string()),
            "ReminderTriggered written to the outbox"
        );
    }

    #[tokio::test]
    async fn skips_a_reminder_that_moved_or_ended() {
        let f = fixture("acct_fire_skip", RecurrencePattern::None).await;
        let fake = FakeOutbound::connected();
        let handler = ReminderFireHandler::new(f.services.clone(), fake.clone());

        let stale = f.reminder.trigger_at - Duration::minutes(1);
        assert!(handler
            .run(&ctx(&f.owner, 1), payload(&f.reminder, stale))
            .await
            .is_ok());

        f.services
            .reminders
            .cancel(&f.owner, f.reminder.id)
            .await
            .unwrap();
        assert!(handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at)
            )
            .await
            .is_ok());

        let unknown = serde_json::json!({ "reminder_id": i64::MAX, "trigger_at": stale });
        assert!(handler.run(&ctx(&f.owner, 1), unknown).await.is_ok());
        assert!(fake.room_sends().is_empty());
    }

    #[tokio::test]
    async fn a_failed_send_mirrors_the_attempt_and_retries() {
        let f = fixture("acct_fire_retry", RecurrencePattern::None).await;
        let handler = ReminderFireHandler::new(
            f.services.clone(),
            FakeOutbound::failing_once("homeserver down"),
        );

        let out = handler
            .run(
                &ctx(&f.owner, 2),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(matches!(out, Err(JobError::Retryable(_))), "{out:?}");
        let after = current(&f).await;
        assert_eq!(after.status, ReminderStatus::Pending);
        assert_eq!(after.retry_count, 2);
        assert_eq!(after.trigger_at, f.reminder.trigger_at);
    }

    #[tokio::test]
    async fn the_last_failed_attempt_marks_the_reminder_failed() {
        let f = fixture("acct_fire_last", RecurrencePattern::None).await;
        let fake = FakeOutbound::failing_once("boom");
        fake.results
            .lock()
            .unwrap()
            .push_back(Err(ChannelError::Transient("boom".to_string())));
        let handler = ReminderFireHandler::new(f.services.clone(), fake);
        let trigger = f.reminder.trigger_at;
        let fourth = handler
            .run(&ctx(&f.owner, 4), payload(&f.reminder, trigger))
            .await;
        assert!(matches!(fourth, Err(JobError::Retryable(_))), "{fourth:?}");

        let out = handler
            .run(
                &ctx(&f.owner, 5),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
        let after = current(&f).await;
        assert_eq!(after.status, ReminderStatus::Failed);
        assert_eq!(
            after.retry_count, 4,
            "the last attempt keeps the mirrored count"
        );
    }

    #[tokio::test]
    async fn the_last_failed_attempt_advances_a_recurring_reminder_rather_than_failing_it() {
        let f = fixture("acct_fire_last_rec", RecurrencePattern::Daily).await;
        let fake = FakeOutbound::failing_once("boom");
        fake.results
            .lock()
            .unwrap()
            .push_back(Err(ChannelError::Transient("boom".to_string())));
        let handler = ReminderFireHandler::new(f.services.clone(), fake);
        let trigger = f.reminder.trigger_at;

        let out = handler
            .run(&ctx(&f.owner, 5), payload(&f.reminder, trigger))
            .await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
        let after = current(&f).await;
        assert_eq!(after.status, ReminderStatus::Pending);
        assert!(after.trigger_at > trigger);
        assert_eq!(after.retry_count, 0);
    }

    #[tokio::test]
    async fn a_disconnected_channel_defers_the_job() {
        let f = fixture("acct_fire_offline", RecurrencePattern::None).await;
        let fake = FakeOutbound::disconnected();
        let handler = ReminderFireHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == CHANNEL_RETRY),
            "{out:?}"
        );
        assert!(fake.room_sends().is_empty());
        let after = current(&f).await;
        assert_eq!(after.status, ReminderStatus::Pending);
        assert_eq!(after.retry_count, 0);
    }

    #[tokio::test]
    async fn a_malformed_payload_is_permanent() {
        let f = fixture("acct_fire_bad", RecurrencePattern::None).await;
        let handler = ReminderFireHandler::new(f.services.clone(), FakeOutbound::connected());

        let out = handler
            .run(&ctx(&f.owner, 1), serde_json::json!({ "reminder_id": "x" }))
            .await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }

    #[tokio::test]
    async fn a_todo_reminder_records_its_todo_as_the_target() {
        let f = fixture_from(
            "acct_fire_todo",
            RecurrencePattern::None,
            Some((ReminderSource::Todo, 42)),
        )
        .await;
        let handler = ReminderFireHandler::new(f.services.clone(), FakeOutbound::connected());

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(out.is_ok(), "{out:?}");
        let target = get_target(&f.pool, "$evt").await.unwrap().unwrap();
        assert_eq!(target.entity_type, "todo");
        assert_eq!(target.entity_id, 42);
    }

    #[tokio::test]
    async fn a_reminder_set_on_telegram_fires_on_telegram() {
        let f = fixture_in("acct_fire_tg", RecurrencePattern::None, None, "telegram:42").await;
        let fake = FakeOutbound::connected();
        let handler = ReminderFireHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(out.is_ok(), "{out:?}");
        let sent = fake.messages();
        assert_eq!(sent.len(), 1);
        assert!(matches!(
            &sent[0].target,
            Target::Conversation(ConversationAddress::Telegram(42))
        ));
        assert_eq!(current(&f).await.status, ReminderStatus::Triggered);
    }

    #[tokio::test]
    async fn a_web_reminder_is_delivered_nowhere_and_succeeds() {
        let f = fixture_in("acct_fire_web", RecurrencePattern::None, None, "web").await;
        let fake = FakeOutbound::connected();
        let handler = ReminderFireHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.messages().is_empty());
        assert_eq!(current(&f).await.status, ReminderStatus::Triggered);
    }

    #[tokio::test]
    async fn a_rejected_send_marks_the_reminder_failed_without_retrying() {
        let f = fixture("acct_fire_rej", RecurrencePattern::None).await;
        let fake = FakeOutbound::connected();
        fake.results
            .lock()
            .unwrap()
            .push_back(Err(ChannelError::Rejected("no such chat".into())));
        let handler = ReminderFireHandler::new(f.services.clone(), fake);

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
        assert_eq!(current(&f).await.status, ReminderStatus::Failed);
    }

    #[tokio::test]
    async fn an_unauthorized_room_marks_the_reminder_failed_without_sending() {
        let f = fixture_in(
            "acct_fire_unauth",
            RecurrencePattern::None,
            None,
            "!victim_dm:test",
        )
        .await;
        let fake = FakeOutbound::connected();
        *fake.auth_error.lock().unwrap() =
            Some("Delivery rejected: owner is not joined to room !victim_dm:test".to_string());
        let handler = ReminderFireHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(
                &ctx(&f.owner, 1),
                payload(&f.reminder, f.reminder.trigger_at),
            )
            .await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
        assert!(fake.room_sends().is_empty());
        assert_eq!(current(&f).await.status, ReminderStatus::Failed);
    }
}
