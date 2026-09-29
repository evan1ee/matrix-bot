//! Scheduler entry points for the portfolio jobs: what the ticks turn into
//! deduplicated `portfolio.check_alerts` / `portfolio.refresh_prices` jobs.

use chrono::{DateTime, Utc};

use super::PortfolioService;
use crate::application::AppError;
use crate::domain::jobs::{JobType, NewJob};
use crate::infrastructure::postgres::repositories;

impl PortfolioService {
    /// Enqueues one alert check per minute bucket, only when an active alert
    /// exists and no earlier check is still queued or running. A check reads
    /// the current prices whenever it runs, so while Matrix is down one
    /// waiting job covers every tick and no backlog of checks builds up.
    pub async fn enqueue_alert_check(&self, now: DateTime<Utc>) -> Result<usize, AppError> {
        if self.list_active_alert_symbols().await?.is_empty() {
            return Ok(0);
        }
        let kind = JobType::PortfolioCheckAlerts.as_str();
        if repositories::jobs::has_open(&self.pool, kind).await? {
            return Ok(0);
        }
        let job = NewJob::portfolio_check_alerts(now);
        Ok(usize::from(
            repositories::jobs::enqueue(&self.pool, &job)
                .await?
                .is_some(),
        ))
    }

    /// Enqueues one price refresh per holding owner per UTC hour.
    pub async fn enqueue_price_refresh(&self, now: DateTime<Utc>) -> Result<usize, AppError> {
        let mut enqueued = 0;
        for owner in self.list_holding_owners().await? {
            let job = NewJob::portfolio_refresh_prices(owner, now);
            if repositories::jobs::enqueue(&self.pool, &job)
                .await?
                .is_some()
            {
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};

    use super::*;
    use crate::application::AppServices;
    use crate::domain::accounts::AccountId;
    use crate::domain::portfolio::alerts::AlertCondition;
    use crate::domain::portfolio::{BuyOutcome, TradeRequest};
    use crate::infrastructure::postgres::repositories::portfolio::history as snapshots;

    async fn services(names: &[&str]) -> (AppServices, Vec<AccountId>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, names).await;
        let owners = names.iter().map(|n| AccountId::from(*n)).collect();
        (AppServices::new(&pool), owners)
    }

    fn noon() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 29, 12, 0, 30).unwrap()
    }

    async fn jobs_of(s: &AppServices, kind: JobType) -> i64 {
        repositories::jobs::count_with_type(s.portfolio.pool(), kind.as_str())
            .await
            .unwrap()
    }

    async fn add_alert(s: &AppServices, owner: &AccountId) {
        s.portfolio
            .create_alert(owner, "AAPL", 20_000, "USD", 2, AlertCondition::Above)
            .await
            .unwrap();
    }

    async fn add_holding(s: &AppServices, owner: &AccountId) {
        s.portfolio
            .deposit_cash(owner, "USD", 2, 1_000_000, None)
            .await
            .unwrap();
        let req = TradeRequest {
            symbol: "AAPL",
            quantity: "1",
            price: "100",
            currency: "USD",
            note: None,
            executed_at: None,
        };
        let out = s.portfolio.buy(owner, req).await.unwrap();
        assert!(matches!(out, BuyOutcome::Ok { .. }));
    }

    #[tokio::test]
    async fn enqueue_alert_check_needs_an_active_alert() {
        let (s, _) = services(&["acct_pf_noalert"]).await;

        assert_eq!(s.portfolio.enqueue_alert_check(noon()).await.unwrap(), 0);
        assert_eq!(jobs_of(&s, JobType::PortfolioCheckAlerts).await, 0);
    }

    #[tokio::test]
    async fn enqueue_alert_check_dedupes_within_a_minute() {
        let (s, o) = services(&["acct_pf_dedupe"]).await;
        add_alert(&s, &o[0]).await;

        let first = s.portfolio.enqueue_alert_check(noon()).await.unwrap();
        let second = s.portfolio.enqueue_alert_check(noon()).await.unwrap();

        assert_eq!((first, second), (1, 0));
        assert_eq!(jobs_of(&s, JobType::PortfolioCheckAlerts).await, 1);
    }

    /// While a check is still waiting (Matrix down), later ticks add nothing:
    /// the waiting job reads current prices when it finally runs.
    #[tokio::test]
    async fn enqueue_alert_check_does_not_pile_up_behind_an_open_job() {
        let (s, o) = services(&["acct_pf_pileup"]).await;
        add_alert(&s, &o[0]).await;

        s.portfolio.enqueue_alert_check(noon()).await.unwrap();
        let later = s
            .portfolio
            .enqueue_alert_check(noon() + Duration::minutes(10))
            .await
            .unwrap();

        assert_eq!(later, 0);
        assert_eq!(jobs_of(&s, JobType::PortfolioCheckAlerts).await, 1);
    }

    #[tokio::test]
    async fn enqueue_price_refresh_is_per_owner_and_hour() {
        let (s, o) = services(&["acct_pf_hold_a", "acct_pf_hold_b", "acct_pf_nohold"]).await;
        add_holding(&s, &o[0]).await;
        add_holding(&s, &o[1]).await;

        let first = s.portfolio.enqueue_price_refresh(noon()).await.unwrap();
        let again = s.portfolio.enqueue_price_refresh(noon()).await.unwrap();
        let next_hour = s
            .portfolio
            .enqueue_price_refresh(noon() + Duration::hours(1))
            .await
            .unwrap();

        assert_eq!((first, again, next_hour), (2, 0, 2));
        assert_eq!(jobs_of(&s, JobType::PortfolioRefreshPrices).await, 4);
    }

    #[tokio::test]
    async fn trigger_due_alerts_with_no_alerts_is_empty() {
        let (s, _) = services(&["acct_pf_trig_none"]).await;

        assert!(s.portfolio.trigger_due_alerts().await.unwrap().is_empty());
    }

    /// The marking is the idempotency guard: a met alert is returned once,
    /// an unmet one stays active.
    #[tokio::test]
    async fn a_met_alert_triggers_exactly_once() {
        let (s, o) = services(&["acct_pf_trig_once"]).await;
        add_alert(&s, &o[0]).await;
        let mut hit = Vec::new();

        s.portfolio
            .trigger_alerts_for_symbol("AAPL", 199.99, &mut hit)
            .await
            .unwrap();
        assert!(hit.is_empty());
        s.portfolio
            .trigger_alerts_for_symbol("AAPL", 200.0, &mut hit)
            .await
            .unwrap();
        s.portfolio
            .trigger_alerts_for_symbol("AAPL", 250.0, &mut hit)
            .await
            .unwrap();

        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].price_minor, 20_000);
        assert_eq!(hit[0].alert.owner_id, o[0]);
        assert!(s
            .portfolio
            .list_active_alert_symbols()
            .await
            .unwrap()
            .is_empty());
    }

    /// Given no holdings, capture still records today's USD snapshot.
    #[tokio::test]
    async fn capturing_records_todays_usd_snapshot() {
        let (s, o) = services(&["acct_pf_snap"]).await;

        s.portfolio
            .capture_current_snapshot_for_owner(&o[0])
            .await
            .unwrap();

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let rows = snapshots::get_snapshots(s.portfolio.pool(), &o[0], "USD", &today)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }
}
