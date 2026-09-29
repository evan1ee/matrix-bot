//! `briefing.send`: generates one account's morning briefing and delivers it
//! to the account's default room.

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{info, warn};

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
        if self
            .services
            .settings
            .get_last_briefing_date(&owner)
            .await
            .as_deref()
            == Some(date.as_str())
        {
            return Ok(());
        }
        let settings = self.services.settings.get_or_default(&owner).await;
        if !settings.briefing_enabled {
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
            .await?;
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
    use chrono::{Timelike, Utc};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::jobs::JobId;
    use crate::domain::rooms::JoinedRoom;
    use crate::events::{EventBroadcaster, SystemEvent};
    use crate::interface::jobs::fake::FakeDelivery;

    const ROOM: &str = "!briefing:test";

    struct Fixture {
        services: AppServices,
        owner: AccountId,
        events: EventBroadcaster,
    }

    /// An active account (no holdings) with briefings `enabled`, and a cached
    /// direct room to deliver to.
    async fn fixture(name: &str, enabled: bool) -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::new(name);
        crate::db::pool::seed_account(&pool, &owner).await;
        let events = EventBroadcaster::new(16);
        let services = AppServices::with_events(&pool, &events);
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
            services,
            owner,
            events,
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

    fn payload() -> serde_json::Value {
        serde_json::json!({ "local_date": "2026-09-29" })
    }

    async fn last_date(f: &Fixture) -> Option<String> {
        f.services.settings.get_last_briefing_date(&f.owner).await
    }

    fn sent_events(rx: &mut tokio::sync::broadcast::Receiver<SystemEvent>) -> Vec<String> {
        let mut out = Vec::new();
        while let Ok(e) = rx.try_recv() {
            if let SystemEvent::MorningBriefingSent { summary, .. } = e {
                out.push(summary);
            }
        }
        out
    }

    /// Given Matrix is disconnected, Then the job waits and the day is open.
    #[tokio::test]
    async fn without_matrix_the_job_waits() {
        let f = fixture("acct_brief_offline", true).await;
        let fake = FakeDelivery::disconnected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler.run(&ctx(Some(f.owner)), payload()).await;

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
            .update_last_briefing_date(&f.owner, "2026-09-29")
            .await
            .unwrap();
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler.run(&ctx(Some(f.owner)), payload()).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_disabled_account_is_skipped() {
        let f = fixture("acct_brief_off", false).await;
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler.run(&ctx(Some(f.owner)), payload()).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends.lock().unwrap().is_empty());
        assert_eq!(last_date(&f).await, None);
    }

    #[tokio::test]
    async fn generates_sends_to_the_default_room_and_records_the_day() {
        let f = fixture("acct_brief_ok", true).await;
        let mut rx = f.events.subscribe();
        let fake = FakeDelivery::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler.run(&ctx(Some(f.owner)), payload()).await;

        assert!(out.is_ok(), "{out:?}");
        let sends = fake.room_sends.lock().unwrap().clone();
        assert_eq!(sends.len(), 1);
        assert_eq!(sends[0].0, ROOM);
        assert!(sends[0].2.contains("早安简报"));
        assert_eq!(last_date(&f).await.as_deref(), Some("2026-09-29"));
        assert_eq!(
            sent_events(&mut rx),
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

        let out = handler.run(&ctx(Some(f.owner)), payload()).await;

        assert!(out.is_ok(), "{out:?}");
        assert_eq!(last_date(&f).await.as_deref(), Some("2026-09-29"));
    }

    #[tokio::test]
    async fn a_job_without_an_account_is_permanent() {
        let f = fixture("acct_brief_noacct", true).await;
        let handler = BriefingSendHandler::new(f.services.clone(), FakeDelivery::connected());

        let out = handler.run(&ctx(None), payload()).await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }
}
