//! `briefing.send`: generates one account's morning briefing and delivers it
//! to the account's default room.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tracing::{error, info, warn};

use super::{parse_payload, Delivery, DeliveryError, MATRIX_RETRY};
use crate::application::briefing::BriefingPayload;
use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::jobs::{BriefingSendPayload, JobType};
use crate::jobs::{JobContext, JobError, JobHandler};

pub struct BriefingSendHandler {
    services: AppServices,
    delivery: Arc<dyn Delivery>,
}

impl BriefingSendHandler {
    pub fn new(services: AppServices, delivery: Arc<dyn Delivery>) -> Self {
        Self { services, delivery }
    }
}

#[async_trait]
impl JobHandler for BriefingSendHandler {
    fn job_type(&self) -> JobType {
        JobType::BriefingSend
    }

    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError> {
        let p: BriefingSendPayload = parse_payload(payload)?;
        let owner = ctx.account_id.ok_or_else(|| {
            JobError::Permanent(anyhow::anyhow!("briefing.send without an account"))
        })?;
        if !self.delivery.is_connected() {
            return Err(JobError::Unavailable(MATRIX_RETRY));
        }
        let date = p.local_date.format("%Y-%m-%d").to_string();
        // Re-check what the scheduler checked: the user may have turned the
        // briefing off, or another run may have sent it, since enqueue.
        // Fallible reads: a database error must retry the job, not fall back
        // to defaults that would silently skip (or wrongly send) a briefing.
        let last_sent = self
            .services
            .settings
            .try_last_briefing_date(&owner)
            .await
            .map_err(JobError::from)?;
        if last_sent.as_deref() == Some(date.as_str()) {
            return Ok(());
        }
        let settings = self
            .services
            .settings
            .get(&owner)
            .await
            .map_err(JobError::from)?;
        if !settings.briefing_enabled {
            return Ok(());
        }
        // A job deferred through an outage can outlive its day; sending it
        // late would double up with the next day's briefing and move
        // last_briefing_date backwards. Strictly today's local date only.
        let today = Utc::now().with_timezone(&settings.tz()).date_naive();
        if p.local_date != today {
            info!(%owner, job_date = %p.local_date, %today, "Stale briefing job dropped");
            return Ok(());
        }
        let briefing = self
            .services
            .briefing
            .generate_briefing(&owner, &settings)
            .await
            .map_err(JobError::Retryable)?;
        let delivered = self.deliver(&owner, &briefing).await?;
        // Frozen: the day is recorded even when the send failed (the old loop
        // did the same), so a briefing is sent at most once per local day.
        self.services
            .briefing
            .record_sent(&owner, p.local_date, &briefing.summary)
            .await
            .map_err(|err| {
                // The message may already be out: a retry would send it again.
                error!(%owner, error = %err, "Briefing sent but the day could not be recorded");
                JobError::Permanent(anyhow::anyhow!("record briefing day: {err}"))
            })?;
        info!(%owner, delivered, "Morning briefing completed");
        Ok(())
    }
}

impl BriefingSendHandler {
    /// Sends to the account's default room. `Ok(false)` covers "no room" and
    /// a failed send, both best-effort as before; only a lost connection
    /// defers the job.
    async fn deliver(
        &self,
        owner: &AccountId,
        briefing: &BriefingPayload,
    ) -> Result<bool, JobError> {
        let Some(room) = self.services.rooms.default_room(owner).await else {
            return Ok(false);
        };
        match self
            .delivery
            .send_to_room(&room, &briefing.plain_text, &briefing.html_content)
            .await
        {
            Ok(_) => Ok(true),
            Err(DeliveryError::NotConnected) => Err(JobError::Unavailable(MATRIX_RETRY)),
            Err(DeliveryError::Failed(reason)) => {
                warn!(%owner, %room, %reason, "Briefing send failed");
                Ok(false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate, Timelike};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::events::EventId;
    use crate::domain::jobs::JobId;
    use crate::domain::rooms::JoinedRoom;
    use crate::interface::jobs::fake::FakeDelivery;

    const ROOM: &str = "!briefing:test";

    struct Fixture {
        pool: sqlx::PgPool,
        services: AppServices,
        owner: AccountId,
    }

    /// An active account (no holdings) with briefings `enabled`, and a cached
    /// direct room to deliver to.
    async fn fixture(name: &str, enabled: bool) -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(name);
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        services
            .settings
            .modify(&owner, |s| {
                s.briefing_enabled = enabled;
                s.briefing_time = format!("{:02}:{:02}", Utc::now().hour(), 0);
                Ok(())
            })
            .await
            .unwrap();
        services
            .rooms
            .remember(&owner, &JoinedRoom::new(ROOM, Some("Bot"), true))
            .await
            .unwrap();
        Fixture {
            pool,
            services,
            owner,
        }
    }

    fn ctx(owner: Option<AccountId>) -> JobContext {
        JobContext {
            job_id: JobId(1),
            account_id: owner,
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        }
    }

    fn payload(date: NaiveDate) -> serde_json::Value {
        serde_json::json!({ "local_date": date })
    }

    fn ymd(date: NaiveDate) -> String {
        date.format("%Y-%m-%d").to_string()
    }

    /// The account's current local date, as the handler computes it.
    async fn today(f: &Fixture) -> NaiveDate {
        let tz = f.services.settings.get_or_default(&f.owner).await.tz();
        Utc::now().with_timezone(&tz).date_naive()
    }

    async fn last_date(f: &Fixture) -> Option<String> {
        f.services.settings.get_last_briefing_date(&f.owner).await
    }

    /// The summaries of this account's stored `MorningBriefingSent` events.
    async fn sent_events(f: &Fixture) -> Vec<String> {
        crate::infrastructure::postgres::repositories::outbox::load_since(
            &f.pool,
            &[f.owner],
            EventId(0),
            50,
        )
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.event_type == "MorningBriefingSent")
        .filter_map(|e| e.json["payload"]["summary"].as_str().map(str::to_string))
        .collect()
    }

    /// Given a database that cannot be read, When the job runs, Then it is
    /// retried, not run against default settings, and nothing is sent or
    /// recorded.
    #[tokio::test]
    async fn a_database_error_retries_instead_of_using_defaults() {
        let f = fixture("acct_brief_dberr", true).await;
        let day = today(&f).await;
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        f.pool.close().await;

        // A closed pool cannot store an event, so nothing was published.
        let out = handler.run(&ctx(Some(f.owner)), payload(day)).await;

        assert!(matches!(out, Err(JobError::Retryable(_))), "{out:?}");
        assert!(fake.room_sends.lock().unwrap().is_empty());
    }

    /// Given Matrix is disconnected, Then the job waits and the day is open.
    #[tokio::test]
    async fn without_matrix_the_job_waits() {
        let f = fixture("acct_brief_offline", true).await;
        let fake = FakeDelivery::disconnected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == MATRIX_RETRY),
            "{out:?}"
        );
        assert_eq!(last_date(&f).await, None);
        assert!(fake.room_sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_already_sent_day_is_skipped() {
        let f = fixture("acct_brief_done", true).await;
        f.services
            .settings
            .update_last_briefing_date(&f.owner, &ymd(today(&f).await))
            .await
            .unwrap();
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_disabled_account_is_skipped() {
        let f = fixture("acct_brief_off", false).await;
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends.lock().unwrap().is_empty());
        assert_eq!(last_date(&f).await, None);
    }

    #[tokio::test]
    async fn generates_sends_to_the_default_room_and_records_the_day() {
        let f = fixture("acct_brief_ok", true).await;
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        let sends = fake.room_sends.lock().unwrap().clone();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, ROOM);
        assert!(sends[0].2.contains("早安简报"));
        assert_eq!(
            last_date(&f).await.as_deref(),
            Some(ymd(today(&f).await).as_str())
        );
        assert_eq!(
            sent_events(&f).await,
            ["0 tasks, 0 habits, 0 bills, 0 holdings"]
        );
    }

    /// Frozen behaviour of the old loop: a failed send still records the day.
    #[tokio::test]
    async fn a_failed_send_still_records_the_day() {
        let f = fixture("acct_brief_fail", true).await;
        let handler = BriefingSendHandler::new(
            f.services.clone(),
            FakeDelivery::failing_once("homeserver down"),
        );

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            last_date(&f).await.as_deref(),
            Some(ymd(today(&f).await).as_str())
        );
        assert_eq!(
            sent_events(&f).await,
            ["0 tasks, 0 habits, 0 bills, 0 holdings"]
        );
    }

    #[tokio::test]
    async fn a_job_without_an_account_is_permanent() {
        let f = fixture("acct_brief_noacct", true).await;
        let handler = BriefingSendHandler::new(f.services.clone(), FakeDelivery::connected());

        let out = handler.run(&ctx(None), payload(today(&f).await)).await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }

    /// A briefing deferred across midnight is stale: nothing is sent,
    /// recorded or published for a date that is no longer today.
    #[tokio::test]
    async fn a_stale_date_is_dropped_without_sending() {
        let f = fixture("acct_brief_stale", true).await;
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        let yesterday = today(&f).await - Duration::days(1);

        let out = handler.run(&ctx(Some(f.owner)), payload(yesterday)).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends.lock().unwrap().is_empty());
        assert_eq!(last_date(&f).await, None);
        assert!(sent_events(&f).await.is_empty());
    }

    /// Given jobs for two consecutive days that run in the wrong order,
    /// Then last_briefing_date never moves backwards.
    #[tokio::test]
    async fn last_briefing_date_never_moves_backwards() {
        let f = fixture("acct_brief_order", true).await;
        let handler = BriefingSendHandler::new(f.services.clone(), FakeDelivery::connected());
        let now = today(&f).await;

        let newer = handler.run(&ctx(Some(f.owner)), payload(now)).await;
        let older = handler
            .run(&ctx(Some(f.owner)), payload(now - Duration::days(1)))
            .await;

        assert!(newer.is_ok() && older.is_ok(), "{newer:?} {older:?}");
        assert_eq!(last_date(&f).await, Some(ymd(now)));
    }

    /// The connection can drop between the pre-check and the send.
    #[tokio::test]
    async fn a_connection_lost_during_the_send_waits_and_records_nothing() {
        let f = fixture("acct_brief_lost", true).await;
        let fake = FakeDelivery::connected();
        fake.room_results
            .lock()
            .unwrap()
            .push_back(Err(DeliveryError::NotConnected));
        let handler = BriefingSendHandler::new(f.services.clone(), fake);

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == MATRIX_RETRY),
            "{out:?}"
        );
        assert_eq!(last_date(&f).await, None);
    }
}
