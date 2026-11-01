//! `briefing.send`: generates one account's morning briefing and delivers it
//! to the account's default room.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use tracing::{error, info, warn};

use super::{
    briefing_audio, deliverable, message, parse_payload, Outbound, SendFailure, CHANNEL_RETRY,
};
use crate::application::briefing::BriefingPayload;
use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress};
use crate::domain::jobs::{BriefingSendPayload, JobType};
use crate::interface::channels::adapter::ChannelError;
use crate::interface::channels::message::{Attachment, Target};
use crate::jobs::{JobContext, JobError, JobHandler};

pub struct BriefingSendHandler {
    services: AppServices,
    outbound: Arc<dyn Outbound>,
}

impl BriefingSendHandler {
    pub fn new(services: AppServices, outbound: Arc<dyn Outbound>) -> Self {
        Self { services, outbound }
    }

    /// Resolves the account's default room and its deliverable channel target,
    /// filtering out channels that are not configured on this deployment so an
    /// identity from a disabled channel falls back to web.
    pub async fn default_target(
        &self,
        owner: &AccountId,
    ) -> (Option<String>, Option<(ConversationAddress, Channel)>) {
        let room = self.services.rooms.default_room(owner).await;
        let target = room
            .as_deref()
            .and_then(deliverable)
            .filter(|(_, channel)| self.outbound.is_configured(*channel));
        (room, target)
    }

    /// Authorizes and sends `briefing` to `address`.
    pub async fn send_to(
        &self,
        owner: &AccountId,
        address: ConversationAddress,
        briefing: &BriefingPayload,
        audio: Option<Attachment>,
    ) -> Result<(), ChannelError> {
        self.outbound
            .authorize_delivery(owner, &address)
            .await
            .map_err(ChannelError::Rejected)?;
        let mut outgoing = message(
            Target::Conversation(address),
            &briefing.plain_text,
            &briefing.html_content,
        );
        outgoing.content.attachments.extend(audio);
        self.outbound.send(outgoing).await.map(|_| ())
    }

    /// Sends `briefing` to the account's default channel on demand. Returns
    /// `Ok(false)` when no external channel is configured for the account
    /// (web-only delivery), `Ok(true)` when delivered to the default channel,
    /// or `Err` when delivery to a configured default channel fails.
    pub async fn deliver_on_demand(
        &self,
        owner: &AccountId,
        briefing: &BriefingPayload,
    ) -> Result<bool, ChannelError> {
        let (_, target) = self.default_target(owner).await;
        let Some((address, _)) = target else {
            return Ok(false);
        };
        self.send_to(owner, address, briefing, None).await?;
        Ok(true)
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
        // Per channel, so the room's platform being down defers the job before
        // the (costly) generation, as the global Matrix check did.
        let (room, target) = self.default_target(&owner).await;
        if let Some((_, channel)) = &target {
            if !self.outbound.is_connected(*channel) {
                return Err(JobError::Unavailable(CHANNEL_RETRY));
            }
        }
        let briefing = self
            .services
            .briefing
            .generate_briefing(&owner, &settings)
            .await
            .map_err(JobError::Retryable)?;
        // Nothing to attach it to without a room and address, so skip the
        // synthesis; a channel-outage retry reuses the recording it made.
        let audio = match (
            briefing.audio_script.as_deref(),
            room.is_some() && target.is_some(),
        ) {
            (Some(script), true) => {
                briefing_audio::prepare(&self.services, &owner, p.local_date, script).await
            }
            _ => None,
        };
        let delivered = self
            .deliver(
                &owner,
                room.as_deref(),
                target.map(|(address, _)| address),
                &briefing,
                audio,
            )
            .await?;
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
        briefing_audio::prune(&self.services, &owner, p.local_date).await;
        info!(%owner, delivered, "Morning briefing completed");
        Ok(())
    }
}

impl BriefingSendHandler {
    /// Sends to the account's default room. `Ok(false)` covers "no room" and
    /// a failed send, both best-effort as before; a room that is no address
    /// counts as delivered (nothing to do); only a lost connection defers.
    async fn deliver(
        &self,
        owner: &AccountId,
        room: Option<&str>,
        address: Option<ConversationAddress>,
        briefing: &BriefingPayload,
        audio: Option<Attachment>,
    ) -> Result<bool, JobError> {
        let Some(room) = room else {
            return Ok(false);
        };
        let Some(address) = address else {
            return Ok(true);
        };
        match self.send_to(owner, address, briefing, audio).await {
            Ok(()) => Ok(true),
            Err(err) => match SendFailure::from(err) {
                SendFailure::Defer => Err(JobError::Unavailable(CHANNEL_RETRY)),
                SendFailure::Retry(reason) | SendFailure::Permanent(reason) => {
                    warn!(%owner, %room, %reason, "Briefing send failed");
                    Ok(false)
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, NaiveDate, Timelike};
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::channels::Channel;
    use crate::domain::events::EventId;
    use crate::domain::jobs::JobId;
    use crate::domain::rooms::JoinedRoom;
    use crate::interface::channels::adapter::ChannelError;
    use crate::interface::jobs::fake::FakeOutbound;

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
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        f.pool.close().await;

        // A closed pool cannot store an event, so nothing was published.
        let out = handler.run(&ctx(Some(f.owner)), payload(day)).await;

        assert!(matches!(out, Err(JobError::Retryable(_))), "{out:?}");
        assert!(fake.room_sends().is_empty());
    }

    /// Given Matrix is disconnected, Then the job waits and the day is open.
    #[tokio::test]
    async fn a_disconnected_channel_defers_the_job() {
        let f = fixture("acct_brief_offline", true).await;
        let fake = FakeOutbound::disconnected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == CHANNEL_RETRY),
            "{out:?}"
        );
        assert_eq!(last_date(&f).await, None);
        assert!(fake.room_sends().is_empty());
    }

    #[tokio::test]
    async fn an_already_sent_day_is_skipped() {
        let f = fixture("acct_brief_done", true).await;
        f.services
            .settings
            .update_last_briefing_date(&f.owner, &ymd(today(&f).await))
            .await
            .unwrap();
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends().is_empty());
    }

    #[tokio::test]
    async fn a_disabled_account_is_skipped() {
        let f = fixture("acct_brief_off", false).await;
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends().is_empty());
        assert_eq!(last_date(&f).await, None);
    }

    #[tokio::test]
    async fn generates_sends_to_the_default_room_and_records_the_day() {
        let f = fixture("acct_brief_ok", true).await;
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(out.is_ok(), "{out:?}");
        let sends = fake.room_sends();
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
            FakeOutbound::failing_once("homeserver down"),
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
        let handler = BriefingSendHandler::new(f.services.clone(), FakeOutbound::connected());

        let out = handler.run(&ctx(None), payload(today(&f).await)).await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }

    /// A briefing deferred across midnight is stale: nothing is sent,
    /// recorded or published for a date that is no longer today.
    #[tokio::test]
    async fn a_stale_date_is_dropped_without_sending() {
        let f = fixture("acct_brief_stale", true).await;
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        let yesterday = today(&f).await - Duration::days(1);

        let out = handler.run(&ctx(Some(f.owner)), payload(yesterday)).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.room_sends().is_empty());
        assert_eq!(last_date(&f).await, None);
        assert!(sent_events(&f).await.is_empty());
    }

    /// Given jobs for two consecutive days that run in the wrong order,
    /// Then last_briefing_date never moves backwards.
    #[tokio::test]
    async fn last_briefing_date_never_moves_backwards() {
        let f = fixture("acct_brief_order", true).await;
        let handler = BriefingSendHandler::new(f.services.clone(), FakeOutbound::connected());
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
        let fake = FakeOutbound::connected();
        fake.results
            .lock()
            .unwrap()
            .push_back(Err(ChannelError::NotConnected(Channel::Matrix)));
        let handler = BriefingSendHandler::new(f.services.clone(), fake);

        let out = handler
            .run(&ctx(Some(f.owner)), payload(today(&f).await))
            .await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == CHANNEL_RETRY),
            "{out:?}"
        );
        assert_eq!(last_date(&f).await, None);
    }

    #[tokio::test]
    async fn an_unconfigured_default_channel_falls_back_to_web_and_records_the_day() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed("acct_brief_unconf_tg");
        crate::db::pool::seed_account(&pool, &owner).await;
        let services = AppServices::new(&pool);
        services
            .settings
            .modify(&owner, |s| {
                s.briefing_enabled = true;
                s.briefing_time = format!("{:02}:{:02}", Utc::now().hour(), 0);
                Ok(())
            })
            .await
            .unwrap();
        let tg = crate::domain::channels::PlatformIdentity::new(
            Channel::Telegram,
            crate::domain::channels::PlatformUserId::parse(Channel::Telegram, "4242").unwrap(),
            Some("Alice".into()),
        );
        services.identity.bind_for_test(&owner, &tg).await;
        let f = Fixture {
            pool,
            services,
            owner,
        };
        let fake = FakeOutbound::with_configured([Channel::Matrix]);
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        let now = today(&f).await;

        let out = handler.run(&ctx(Some(f.owner)), payload(now)).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.messages().is_empty());
        assert_eq!(last_date(&f).await, Some(ymd(now)));
        assert_eq!(
            sent_events(&f).await,
            ["0 tasks, 0 habits, 0 bills, 0 holdings"]
        );
    }

    /// Serves `/audio/speech` with `status` and points the TTS setting at it.
    async fn tts_server(f: &Fixture, status: axum::http::StatusCode) {
        use crate::domain::config::{SettingKey, SettingValue};
        let app = axum::Router::new().route(
            "/audio/speech",
            axum::routing::post(move || async move { (status, "ogg-bytes") }),
        );
        let base = crate::infrastructure::inference::test_support::spawn_server(app).await;
        f.services
            .app_config
            .update_setting(
                &f.owner,
                SettingKey::InferenceTtsBaseUrl,
                SettingValue::String(base),
            )
            .await
            .unwrap();
    }

    /// Given working TTS, Then the message carries the audio, the recording is
    /// stored and recorded for the day.
    #[tokio::test]
    async fn the_briefing_carries_its_recording() {
        let f = fixture("acct_brief_audio", true).await;
        tts_server(&f, axum::http::StatusCode::OK).await;
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        let day = today(&f).await;

        let out = handler.run(&ctx(Some(f.owner)), payload(day)).await;

        assert!(out.is_ok(), "{out:?}");
        let sent = fake.messages();
        assert_eq!(sent.len(), 1);
        let audio = &sent[0].content.attachments;
        assert_eq!(audio.len(), 1);
        assert_eq!(
            audio[0].kind,
            crate::interface::channels::message::AttachmentKind::Audio
        );
        assert_eq!(audio[0].filename, "morning-briefing.ogg");
        let key = f
            .services
            .briefing
            .audio_key(&f.owner, day)
            .await
            .unwrap()
            .unwrap();
        let stored = f
            .services
            .storage
            .get(&crate::domain::storage::StorageKey::parse(&key).unwrap())
            .await
            .unwrap();
        assert_eq!(stored.as_ref(), audio[0].bytes.as_slice());
    }

    /// Given a recording already kept for the day (an earlier attempt), When
    /// the job prepares audio again, Then it reuses it without calling TTS.
    #[tokio::test]
    async fn a_retry_reuses_the_kept_recording() {
        use crate::domain::config::{SettingKey, SettingValue};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let hits = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let app = axum::Router::new().route(
            "/audio/speech",
            axum::routing::post(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    (axum::http::StatusCode::OK, "ogg-bytes")
                }
            }),
        );
        let base = crate::infrastructure::inference::test_support::spawn_server(app).await;
        let f = fixture("acct_brief_retry_audio", true).await;
        f.services
            .app_config
            .update_setting(
                &f.owner,
                SettingKey::InferenceTtsBaseUrl,
                SettingValue::String(base),
            )
            .await
            .unwrap();
        let day = today(&f).await;

        let first = briefing_audio::prepare(&f.services, &f.owner, day, "Good morning")
            .await
            .expect("first attempt synthesizes");
        let second = briefing_audio::prepare(&f.services, &f.owner, day, "Good morning")
            .await
            .expect("retry reuses the recording");

        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(first.bytes, second.bytes);
    }

    /// Given a failing TTS service, Then the text still goes out, the day is
    /// recorded and no recording exists.
    #[tokio::test]
    async fn a_failing_tts_service_costs_only_the_audio() {
        let f = fixture("acct_brief_tts_down", true).await;
        tts_server(&f, axum::http::StatusCode::INTERNAL_SERVER_ERROR).await;
        let fake = FakeOutbound::connected();
        let handler = BriefingSendHandler::new(f.services.clone(), fake.clone());
        let day = today(&f).await;

        let out = handler.run(&ctx(Some(f.owner)), payload(day)).await;

        assert!(out.is_ok(), "{out:?}");
        let sent = fake.messages();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].content.attachments.is_empty());
        assert_eq!(
            f.services.briefing.audio_key(&f.owner, day).await.unwrap(),
            None
        );
        assert_eq!(last_date(&f).await, Some(ymd(day)));
    }

    /// Given a recording older than 30 days, When the day's briefing runs,
    /// Then that recording and its object are gone and a recent one stays.
    #[tokio::test]
    async fn old_recordings_are_pruned_by_the_next_briefing() {
        use crate::domain::storage::{StorageCategory, StorageKey};
        let f = fixture("acct_brief_prune", true).await;
        let day = today(&f).await;
        let old_key = StorageKey::generate(&f.owner, StorageCategory::Audio, "ogg");
        f.services
            .storage
            .put(&old_key, bytes::Bytes::from_static(b"old"), "audio/ogg")
            .await
            .unwrap();
        let old_day = day - Duration::days(31);
        let recent_day = day - Duration::days(5);
        f.services
            .briefing
            .record_audio(&f.owner, old_day, old_key.as_str())
            .await
            .unwrap();
        f.services
            .briefing
            .record_audio(&f.owner, recent_day, "accounts/x/audio/recent.ogg")
            .await
            .unwrap();
        let handler = BriefingSendHandler::new(f.services.clone(), FakeOutbound::connected());

        let out = handler.run(&ctx(Some(f.owner)), payload(day)).await;

        assert!(out.is_ok(), "{out:?}");
        assert_eq!(
            f.services
                .briefing
                .audio_key(&f.owner, old_day)
                .await
                .unwrap(),
            None
        );
        assert!(f.services.storage.get(&old_key).await.is_err());
        assert!(f
            .services
            .briefing
            .audio_key(&f.owner, recent_day)
            .await
            .unwrap()
            .is_some());
    }
}
