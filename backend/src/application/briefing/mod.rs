use chrono::{DateTime, NaiveDate, Utc};

use crate::application::accounts::AccountService;
use crate::application::settings::SettingsService;
use crate::application::AppError;
use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::domain::briefing::AUDIO_RETENTION_DAYS;
use crate::domain::events::SystemEvent;
use crate::domain::jobs::NewJob;
use crate::domain::settings::{briefing_due_date, UserSettings};
use crate::infrastructure::postgres::repositories::{self, outbox};

pub mod audio_script;
pub mod generator;
pub mod models;
pub mod ticker;

pub use models::{BriefingPayload, BriefingSummary, TickerPerformance};

#[derive(Clone)]
pub struct BriefingService {
    pool: DbPool,
    settings: SettingsService,
    accounts: AccountService,
}

impl BriefingService {
    pub fn new(pool: DbPool, settings: SettingsService, accounts: AccountService) -> Self {
        Self {
            pool,
            settings,
            accounts,
        }
    }

    pub async fn generate_briefing(
        &self,
        owner_id: &AccountId,
        settings: &UserSettings,
    ) -> anyhow::Result<BriefingPayload> {
        generator::generate_briefing(&self.pool, owner_id, settings).await
    }

    /// Enqueues a `briefing.send` job for every active account whose briefing
    /// is due at `now`. Idempotent: the dedupe key is `<account>:<local date>`,
    /// so at most one job ever exists per account and local day. A job that
    /// ends `failed` therefore blocks that day's briefing, by design.
    pub async fn enqueue_due(&self, now: DateTime<Utc>) -> Result<usize, AppError> {
        let mut enqueued = 0;
        for owner in self.accounts.list_active_ids().await? {
            let settings = self.settings.get_or_default(&owner).await;
            let last_sent = self.settings.get_last_briefing_date(&owner).await;
            let Some(date) = briefing_due_date(&settings, now, last_sent.as_deref()) else {
                continue;
            };
            let job = NewJob::briefing_send(owner, date);
            if repositories::jobs::enqueue(&self.pool, &job)
                .await?
                .is_some()
            {
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }

    /// Records the day as sent and publishes `MorningBriefingSent` in the
    /// same transaction as the `last_briefing_date` write.
    pub async fn record_sent(
        &self,
        owner: &AccountId,
        local_date: NaiveDate,
        summary: &BriefingSummary,
    ) -> Result<(), AppError> {
        let date = local_date.format("%Y-%m-%d").to_string();
        let mut tx = self.pool.begin().await?;
        repositories::settings::update_last_briefing_date(&mut *tx, owner, &date).await?;
        outbox::publish(&mut tx, &sent_event(owner, date, summary)).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The stored recording of `local_date`'s briefing, if one was kept.
    pub async fn audio_key(
        &self,
        owner: &AccountId,
        local_date: NaiveDate,
    ) -> Result<Option<String>, AppError> {
        Ok(repositories::briefing_audio::find_key(&self.pool, owner, local_date).await?)
    }

    /// Points `local_date` at the stored recording `key`. Returns the key of
    /// the recording it replaced, so the caller can delete that object.
    pub async fn record_audio(
        &self,
        owner: &AccountId,
        local_date: NaiveDate,
        key: &str,
    ) -> Result<Option<String>, AppError> {
        let mut tx = self.pool.begin().await?;
        let previous = repositories::briefing_audio::find_key(&mut *tx, owner, local_date)
            .await?
            .filter(|old| old != key);
        repositories::briefing_audio::upsert(&mut *tx, owner, local_date, key).await?;
        tx.commit().await?;
        Ok(previous)
    }

    /// Forgets the owner's recordings older than the retention window and
    /// returns their keys for the caller to delete from storage.
    pub async fn prune_audio(
        &self,
        owner: &AccountId,
        today: NaiveDate,
    ) -> Result<Vec<String>, AppError> {
        let cutoff = today - chrono::Duration::days(AUDIO_RETENTION_DAYS);
        Ok(repositories::briefing_audio::delete_before(&self.pool, owner, cutoff).await?)
    }

    /// Announces an on-demand briefing (REST trigger, `!briefing`) to the
    /// event stream. Does not touch `last_briefing_date`: a manual run must
    /// not suppress the scheduled one.
    pub async fn announce_manual(
        &self,
        owner: &AccountId,
        payload: &BriefingPayload,
    ) -> Result<(), AppError> {
        let delivered_at = Utc::now().format("%Y-%m-%d").to_string();
        let mut tx = self.pool.begin().await?;
        outbox::publish(&mut tx, &sent_event(owner, delivered_at, &payload.summary)).await?;
        tx.commit().await?;
        Ok(())
    }
}

fn sent_event(owner: &AccountId, delivered_at: String, summary: &BriefingSummary) -> SystemEvent {
    SystemEvent::MorningBriefingSent {
        owner_id: *owner,
        delivered_at,
        summary: format!(
            "{} tasks, {} habits, {} bills, {} holdings",
            summary.pending_tasks_count,
            summary.habits_completed_count,
            summary.upcoming_bills_count,
            summary.holdings_count
        ),
    }
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;
    use crate::application::AppServices;
    use crate::domain::events::EventId;
    use crate::domain::jobs::JobStatus;

    struct Fixture {
        pool: DbPool,
        services: AppServices,
        owner: AccountId,
    }

    async fn setup(name: &str) -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(name);
        crate::db::pool::seed_account(&pool, &owner).await;
        Fixture {
            services: AppServices::new(&pool),
            pool,
            owner,
        }
    }

    /// Configures the briefing for minute 0 of the current UTC hour and
    /// returns an instant one minute into that window.
    async fn configure(f: &Fixture, enabled: bool) -> DateTime<Utc> {
        let now = Utc::now()
            .with_minute(1)
            .and_then(|t| t.with_second(0))
            .unwrap_or_else(Utc::now);
        f.services
            .settings
            .modify(&f.owner, |s| {
                s.briefing_enabled = enabled;
                s.briefing_time = format!("{:02}:00", now.hour());
                s.timezone = "UTC".to_string();
                Ok(())
            })
            .await
            .unwrap();
        now
    }

    /// The status of this account's briefing job for `now`'s local day.
    async fn job_status(f: &Fixture, now: DateTime<Utc>) -> Option<JobStatus> {
        let job = NewJob::briefing_send(f.owner, now.date_naive());
        let key = job.dedupe_key().unwrap_or_default();
        repositories::jobs::status_by_key(&f.pool, job.job_type().as_str(), key)
            .await
            .unwrap()
    }

    /// Given an active account with briefing enabled at the current UTC time,
    /// When enqueue_due runs twice, Then one briefing.send job exists.
    #[tokio::test]
    async fn enqueue_due_enqueues_once_per_local_day() {
        let f = setup("acct_brief_enq").await;
        let now = configure(&f, true).await;

        let first = f.services.briefing.enqueue_due(now).await.unwrap();
        let second = f.services.briefing.enqueue_due(now).await.unwrap();

        assert_eq!(first, 1);
        assert_eq!(second, 0, "the second tick deduplicates");
        assert_eq!(job_status(&f, now).await, Some(JobStatus::Pending));
    }

    /// Given a disabled account, Then nothing is enqueued for it.
    #[tokio::test]
    async fn enqueue_due_skips_disabled_accounts() {
        let f = setup("acct_brief_disabled").await;
        let now = configure(&f, false).await;

        f.services.briefing.enqueue_due(now).await.unwrap();

        assert_eq!(job_status(&f, now).await, None);
    }

    /// The stored `MorningBriefingSent` events of the fixture's owner.
    async fn sent_events(f: &Fixture) -> Vec<serde_json::Value> {
        repositories::outbox::load_since(&f.pool, &[f.owner], EventId(0), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.event_type == "MorningBriefingSent")
            .map(|e| e.json)
            .collect()
    }

    fn summary() -> BriefingSummary {
        BriefingSummary {
            pending_tasks_count: 3,
            habits_completed_count: 2,
            habits_total_count: 5,
            upcoming_bills_count: 1,
            holdings_count: 4,
            budgets_attention_count: 0,
        }
    }

    /// When a manual briefing is announced, Then MorningBriefingSent carries
    /// today's date and the formatted summary.
    #[tokio::test]
    async fn announce_manual_publishes_the_summary_line() {
        let f = setup("acct_brief_manual").await;
        let payload = BriefingPayload {
            owner_id: f.owner.to_string(),
            html_content: String::new(),
            plain_text: String::new(),
            summary: summary(),
            tickers: Vec::new(),
            audio_script: None,
        };

        f.services
            .briefing
            .announce_manual(&f.owner, &payload)
            .await
            .unwrap();

        let events = sent_events(&f).await;
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["payload"]["summary"],
            "3 tasks, 2 habits, 1 bills, 4 holdings"
        );
        assert_eq!(
            events[0]["payload"]["delivered_at"],
            Utc::now().format("%Y-%m-%d").to_string()
        );
    }

    /// When record_sent runs, Then last_briefing_date is the date and
    /// MorningBriefingSent is published with the summary line.
    #[tokio::test]
    async fn record_sent_marks_the_day_and_publishes() {
        let f = setup("acct_brief_record").await;
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();

        f.services
            .briefing
            .record_sent(&f.owner, date, &summary())
            .await
            .unwrap();

        assert_eq!(
            f.services
                .settings
                .get_last_briefing_date(&f.owner)
                .await
                .as_deref(),
            Some("2026-09-29")
        );
        let events = sent_events(&f).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["payload"]["delivered_at"], "2026-09-29");
        assert_eq!(
            events[0]["payload"]["summary"],
            "3 tasks, 2 habits, 1 bills, 4 holdings"
        );
    }
}
