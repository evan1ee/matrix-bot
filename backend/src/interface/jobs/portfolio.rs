//! `portfolio.check_alerts` and `portfolio.refresh_prices`: alert evaluation
//! with DMs to linked identities, and the hourly per-owner price refresh.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use super::{Delivery, MATRIX_RETRY};
use crate::application::portfolio::TriggeredAlert;
use crate::application::{AppError, AppServices};
use crate::domain::jobs::JobType;
use crate::domain::portfolio::alerts::{AlertCondition, PriceAlert};
use crate::jobs::{JobContext, JobError, JobHandler};
use crate::utils::html::html_escape;

/// Formats a clean, readable notification for a triggered alert (both plain and HTML).
pub fn format_alert_notification(alert: &PriceAlert, current_price_minor: i64) -> (String, String) {
    let cond = alert.parsed_condition();
    let exp = alert.currency_exponent as i32;
    let target = (alert.target_price_minor as f64) / 10f64.powi(exp);
    let current = (current_price_minor as f64) / 10f64.powi(exp);

    let (icon, direction_word) = match cond {
        AlertCondition::Above => ("🚀", "risen above"),
        AlertCondition::Below => ("⚠️", "dropped below"),
    };

    let diff_pct = if target > 0.0 {
        ((current - target) / target) * 100.0
    } else {
        0.0
    };

    let plain = format!(
        "{} Price Alert: {} has {} target {:.2} {}! Current price: {:.2} {} ({:+.2}%)",
        icon,
        alert.symbol,
        direction_word,
        target,
        alert.currency,
        current,
        alert.currency,
        diff_pct
    );

    let html = format!(
        r#"<div style="font-family:sans-serif;padding:8px 0;">
  <p><strong>{} Price Alert: {}</strong></p>
  <p>{} has <strong>{}</strong> your target of <strong>{:.2} {}</strong>.</p>
  <p>Current Market Price: <strong>{:.2} {}</strong> (<em>{:+.2}%</em>)</p>
</div>"#,
        icon,
        html_escape(&alert.symbol),
        html_escape(&alert.symbol),
        direction_word,
        target,
        html_escape(&alert.currency),
        current,
        html_escape(&alert.currency),
        diff_pct
    );

    (plain, html)
}

pub struct CheckAlertsHandler {
    services: AppServices,
    delivery: Arc<dyn Delivery>,
}

impl CheckAlertsHandler {
    pub fn new(services: AppServices, delivery: Arc<dyn Delivery>) -> Self {
        Self { services, delivery }
    }

    /// DMs every linked identity; a failed send is logged and skipped. The
    /// alert is already marked triggered, so nothing here may fail the job
    /// into a retry that would double-notify.
    pub(crate) async fn notify(&self, hit: &TriggeredAlert, mxids: Vec<String>) {
        let (plain, html) = format_alert_notification(&hit.alert, hit.price_minor);
        for mxid in mxids {
            if let Err(err) = self.delivery.send_dm(&mxid, &plain, &html).await {
                warn!(%mxid, error = %err, "Alert DM not sent");
            }
        }
    }
}

#[async_trait]
impl JobHandler for CheckAlertsHandler {
    fn job_type(&self) -> JobType {
        JobType::PortfolioCheckAlerts
    }

    async fn run(&self, _ctx: &JobContext, _payload: serde_json::Value) -> Result<(), JobError> {
        // Checked before evaluating: an alert is marked triggered before its
        // DM (the idempotency guard), so evaluating while Matrix is down
        // would consume alerts nobody is told about.
        if !self.delivery.is_connected() {
            return Err(JobError::Unavailable(MATRIX_RETRY));
        }
        let symbols = self.services.portfolio.list_active_alert_symbols().await?;
        let portfolio = self.services.portfolio.clone();
        self.check_symbols(symbols, |symbol| {
            let portfolio = portfolio.clone();
            async move { portfolio.trigger_symbol_alerts(&symbol).await }
        })
        .await
    }
}

impl CheckAlertsHandler {
    /// Evaluates one symbol at a time and sends its DMs before the next
    /// symbol's price is fetched: a timeout or shutdown mid-check (with a
    /// single attempt) then loses at most the symbol in flight, not alerts
    /// already marked for earlier ones.
    pub(crate) async fn check_symbols<F, Fut>(
        &self,
        symbols: Vec<String>,
        mut trigger: F,
    ) -> Result<(), JobError>
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = Result<Vec<TriggeredAlert>, AppError>>,
    {
        for symbol in symbols {
            for hit in trigger(symbol).await? {
                let owner = &hit.alert.owner_id;
                let mxids = match self.services.accounts.linked_mxids(owner).await {
                    Ok(mxids) => mxids,
                    Err(err) => {
                        warn!(%owner, error = %err, "Could not read linked identities for an alert");
                        Vec::new()
                    }
                };
                self.notify(&hit, mxids).await;
            }
        }
        Ok(())
    }
}

pub struct RefreshPricesHandler {
    services: AppServices,
}

impl RefreshPricesHandler {
    pub fn new(services: AppServices) -> Self {
        Self { services }
    }
}

#[async_trait]
impl JobHandler for RefreshPricesHandler {
    fn job_type(&self) -> JobType {
        JobType::PortfolioRefreshPrices
    }

    async fn run(&self, ctx: &JobContext, _payload: serde_json::Value) -> Result<(), JobError> {
        let owner = ctx.account_id.ok_or_else(|| {
            JobError::Permanent(anyhow::anyhow!("refresh_prices without an account"))
        })?;
        let summary = self.services.portfolio.refresh_all_for_owner(&owner).await;
        if !summary.updated.is_empty() {
            debug!(%owner, updated = summary.updated.len(), "Scheduled price refresh");
        }
        // Idempotent per day (an upsert), so a retry after a failure is safe.
        self.services
            .portfolio
            .capture_current_snapshot_for_owner(&owner)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::jobs::JobId;
    use crate::domain::portfolio::alerts::AlertCondition;
    use crate::interface::jobs::fake::FakeDelivery;
    use crate::interface::jobs::DeliveryError;

    async fn services(names: &[&str]) -> (AppServices, Vec<AccountId>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, names).await;
        let owners = names.iter().map(|n| AccountId::from_seed(n)).collect();
        (AppServices::new(&pool), owners)
    }

    fn ctx(owner: Option<AccountId>) -> JobContext {
        JobContext {
            job_id: JobId(1),
            account_id: owner,
            attempt: 1,
            max_attempts: 1,
            cancel: CancellationToken::new(),
        }
    }

    fn alert(condition: &str, target_minor: i64) -> PriceAlert {
        PriceAlert {
            id: 7,
            owner_id: AccountId::from_seed("acct_pf_fmt"),
            symbol: "AAPL".into(),
            target_price_minor: target_minor,
            currency: "USD".into(),
            currency_exponent: 2,
            condition: condition.into(),
            is_triggered: false,
            triggered_at: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn the_notification_text_is_frozen() {
        let (plain, html) = format_alert_notification(&alert("above", 20_000), 21_000);
        assert_eq!(
            plain,
            "🚀 Price Alert: AAPL has risen above target 200.00 USD! Current price: 210.00 USD (+5.00%)"
        );
        assert!(html.contains("<strong>risen above</strong>"));
        let (plain, _) = format_alert_notification(&alert("below", 20_000), 19_000);
        assert!(plain.contains("dropped below") && plain.contains("(-5.00%)"));
    }

    #[tokio::test]
    async fn check_alerts_waits_without_matrix() {
        let (s, _) = services(&["acct_pf_h_offline"]).await;
        let handler = CheckAlertsHandler::new(s, FakeDelivery::disconnected());

        let out = handler.run(&ctx(None), serde_json::json!({})).await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == MATRIX_RETRY),
            "{out:?}"
        );
    }

    /// While Matrix is down the handler never reaches the marking step, so
    /// an active alert is not consumed without a notification.
    #[tokio::test]
    async fn check_alerts_without_matrix_leaves_alerts_active() {
        let (s, o) = services(&["acct_pf_h_keep"]).await;
        s.portfolio
            .create_alert(&o[0], "AAPL", 1, "USD", 2, AlertCondition::Above)
            .await
            .unwrap();
        let handler = CheckAlertsHandler::new(s.clone(), FakeDelivery::disconnected());

        let _ = handler.run(&ctx(None), serde_json::json!({})).await;

        assert_eq!(
            s.portfolio.list_active_alert_symbols().await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn check_alerts_with_no_alerts_sends_nothing() {
        let (s, _) = services(&["acct_pf_h_none"]).await;
        let fake = FakeDelivery::connected();
        let handler = CheckAlertsHandler::new(s, fake.clone());

        let out = handler.run(&ctx(None), serde_json::json!({})).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.dms.lock().unwrap().is_empty());
    }

    /// Every linked identity gets the DM, and one failing send does not stop
    /// the rest.
    #[tokio::test]
    async fn a_triggered_alert_is_sent_to_every_linked_mxid() {
        let (s, _) = services(&["acct_pf_h_dm"]).await;
        let fake = FakeDelivery::connected();
        fake.dm_results
            .lock()
            .unwrap()
            .push_back(Err(DeliveryError::Failed("homeserver down".into())));
        let handler = CheckAlertsHandler::new(s, fake.clone());
        let hit = TriggeredAlert {
            alert: alert("above", 20_000),
            price_minor: 21_000,
        };

        handler
            .notify(&hit, vec!["@a:test".into(), "@b:test".into()])
            .await;

        let dms = fake.dms.lock().unwrap().clone();
        let to: Vec<&str> = dms.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(to, ["@a:test", "@b:test"]);
        assert!(dms[0].1.contains("Price Alert: AAPL"));
    }

    /// The DMs for symbol A are out before symbol B is even evaluated, so a
    /// timeout or shutdown during B cannot lose A's already-marked alerts.
    #[tokio::test]
    async fn dms_for_one_symbol_are_sent_before_the_next_is_evaluated() {
        let (s, _) = services(&["acct_pf_h_order"]).await;
        let owner = s
            .accounts
            .resolve_or_provision("@order:test")
            .await
            .unwrap();
        let fake = FakeDelivery::connected();
        let handler = CheckAlertsHandler::new(s, fake.clone());
        let seen_at_b = Arc::new(std::sync::Mutex::new(None));

        let out = handler
            .check_symbols(vec!["A".into(), "B".into()], |symbol| {
                let fake = fake.clone();
                let seen_at_b = seen_at_b.clone();
                let mut hit = alert("above", 20_000);
                hit.owner_id = owner;
                async move {
                    if symbol == "B" {
                        *seen_at_b.lock().unwrap() = Some(fake.dms.lock().unwrap().len());
                        return Ok(Vec::new());
                    }
                    Ok(vec![TriggeredAlert {
                        alert: hit,
                        price_minor: 21_000,
                    }])
                }
            })
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert_eq!(*seen_at_b.lock().unwrap(), Some(1));
    }

    /// An owner without holdings still runs cleanly. That the snapshot row is
    /// written is proven in application::portfolio::scheduling, because
    /// get_history cannot observe it.
    #[tokio::test]
    async fn refresh_prices_succeeds_for_an_owner_without_holdings() {
        let (s, o) = services(&["acct_pf_h_snap"]).await;
        let handler = RefreshPricesHandler::new(s);

        let out = handler.run(&ctx(Some(o[0])), serde_json::json!({})).await;

        assert!(out.is_ok(), "{out:?}");
    }

    #[tokio::test]
    async fn refresh_prices_without_an_account_is_permanent() {
        let (s, _) = services(&["acct_pf_h_noacct"]).await;
        let handler = RefreshPricesHandler::new(s);

        let out = handler.run(&ctx(None), serde_json::json!({})).await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }
}
