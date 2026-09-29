use chrono::{DateTime, NaiveDate, Utc};

use crate::application::accounts::AccountService;
use crate::application::settings::SettingsService;
use crate::application::AppError;
use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::domain::jobs::NewJob;
use crate::domain::settings::{briefing_due_date, UserSettings};
use crate::events::{EventBroadcaster, SystemEvent};
use crate::infrastructure::postgres::repositories;

pub mod generator;
pub mod models;
pub mod ticker;

pub use models::{BriefingPayload, BriefingSummary, TickerPerformance};

#[derive(Clone)]
pub struct BriefingService {
    pool: DbPool,
    events: EventBroadcaster,
    settings: SettingsService,
    accounts: AccountService,
}

impl BriefingService {
    pub fn new(
        pool: DbPool,
        events: EventBroadcaster,
        settings: SettingsService,
        accounts: AccountService,
    ) -> Self {
        Self {
            pool,
            events,
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

    /// Records the day as sent and publishes `MorningBriefingSent`.
    pub async fn record_sent(
        &self,
        owner: &AccountId,
        local_date: NaiveDate,
        summary: &BriefingSummary,
    ) -> Result<(), AppError> {
        let date = local_date.format("%Y-%m-%d").to_string();
        self.settings
            .update_last_briefing_date(owner, &date)
            .await?;
        self.events.send(SystemEvent::MorningBriefingSent {
            owner_id: owner.to_string(),
            delivered_at: date,
            summary: format!(
                "{} tasks, {} habits, {} bills, {} holdings",
                summary.pending_tasks_count,
                summary.habits_completed_count,
                summary.upcoming_bills_count,
                summary.holdings_count
            ),
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use chrono::Timelike;

    use super::*;
    use crate::application::AppServices;
    use crate::domain::jobs::JobStatus;

    struct Fixture {
        pool: DbPool,
        services: AppServices,
        owner: AccountId,
        events: EventBroadcaster,
    }

    async fn setup(name: &str) -> Fixture {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::new(name);
        crate::db::pool::seed_account(&pool, &owner).await;
        let events = EventBroadcaster::new(16);
        Fixture {
            services: AppServices::with_events(&pool, &events),
            pool,
            owner,
            events,
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

    /// When record_sent runs, Then last_briefing_date is the date and
    /// MorningBriefingSent is published with the summary line.
    #[tokio::test]
    async fn record_sent_marks_the_day_and_publishes() {
        let f = setup("acct_brief_record").await;
        let mut rx = f.events.subscribe();
        let date = NaiveDate::from_ymd_opt(2026, 9, 29).unwrap();
        let summary = BriefingSummary {
            pending_tasks_count: 3,
            habits_completed_count: 2,
            habits_total_count: 5,
            upcoming_bills_count: 1,
            holdings_count: 4,
        };

        f.services
            .briefing
            .record_sent(&f.owner, date, &summary)
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
        let mut got = None;
        while let Ok(e) = rx.try_recv() {
            if let SystemEvent::MorningBriefingSent {
                owner_id,
                delivered_at,
                summary,
            } = e
            {
                got = Some((owner_id, delivered_at, summary));
            }
        }
        assert_eq!(
            got,
            Some((
                f.owner.to_string(),
                "2026-09-29".to_string(),
                "3 tasks, 2 habits, 1 bills, 4 holdings".to_string()
            ))
        );
    }
}
