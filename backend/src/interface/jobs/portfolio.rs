//! `portfolio.check_alerts` and `portfolio.refresh_prices`: alert evaluation
//! with DMs to linked identities, and the hourly per-owner price refresh.

use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use super::dm::{dm_linked_identities, owner_ready};
use super::{Outbound, CHANNEL_RETRY};
use crate::application::portfolio::TriggeredAlert;
use crate::application::{AppError, AppServices};
use crate::domain::channels::LinkedIdentity;
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

use std::collections::HashSet;

pub struct CheckAlertsHandler {
    services: AppServices,
    outbound: Arc<dyn Outbound>,
}

impl CheckAlertsHandler {
    pub fn new(services: AppServices, outbound: Arc<dyn Outbound>) -> Self {
        Self { services, outbound }
    }

    /// Partitions the owners of active alerts on `symbol` into those ready to
    /// be evaluated now and whether any owner had to be deferred because all
    /// of their configured channel identities are currently disconnected.
    pub(crate) async fn ready_owners_for_symbol(
        &self,
        symbol: &str,
    ) -> Result<(HashSet<crate::domain::accounts::AccountId>, bool), AppError> {
        let alerts = self
            .services
            .portfolio
            .list_active_alerts_for_symbol(symbol)
            .await?;
        let mut ready = HashSet::new();
        let mut deferred = HashSet::new();
        for alert in alerts {
            let owner = alert.owner_id;
            if ready.contains(&owner) || deferred.contains(&owner) {
                continue;
            }
            if owner_ready(&self.services, self.outbound.as_ref(), &owner).await? {
                ready.insert(owner);
            } else {
                deferred.insert(owner);
            }
        }
        let any_deferred = !deferred.is_empty();
        Ok((ready, any_deferred))
    }

    /// DMs every linked identity on a connected channel; a failed send is
    /// logged and skipped. The alert is already marked triggered, so nothing
    /// here may fail the job into a retry that would double-notify.
    pub(crate) async fn notify_identities(
        &self,
        hit: &TriggeredAlert,
        identities: Vec<LinkedIdentity>,
    ) {
        let (plain, html) = format_alert_notification(&hit.alert, hit.price_minor);
        dm_linked_identities(self.outbound.as_ref(), identities, &plain, &html).await;
    }

    #[cfg(test)]
    pub(crate) async fn notify(&self, hit: &TriggeredAlert, mxids: Vec<String>) {
        let identities = mxids
            .into_iter()
            .map(|mxid| {
                LinkedIdentity::new(
                    crate::domain::channels::Channel::Matrix,
                    mxid,
                    None,
                    chrono::Utc::now(),
                )
            })
            .collect();
        self.notify_identities(hit, identities).await;
    }
}

#[async_trait]
impl JobHandler for CheckAlertsHandler {
    fn job_type(&self) -> JobType {
        JobType::PortfolioCheckAlerts
    }

    async fn run(&self, _ctx: &JobContext, _payload: serde_json::Value) -> Result<(), JobError> {
        // Checked before evaluating: an alert is marked triggered before its
        // DM (the idempotency guard), so evaluating while all configured
        // channels are down would consume alerts nobody is told about.
        if !self.outbound.is_ready_for_alerts() {
            return Err(JobError::Unavailable(CHANNEL_RETRY));
        }
        let symbols = self.services.portfolio.list_active_alert_symbols().await?;
        let portfolio = self.services.portfolio.clone();
        self.evaluate_symbols(symbols, |symbol, ready_owners| {
            let portfolio = portfolio.clone();
            async move {
                portfolio
                    .trigger_symbol_alerts_for_owners(&symbol, &ready_owners)
                    .await
            }
        })
        .await
    }
}

impl CheckAlertsHandler {
    /// Evaluates one symbol at a time for owners whose channels are ready,
    /// sends its DMs before the next symbol's price is fetched, and defers
    /// if any active alert's owner is waiting on a disconnected channel.
    pub(crate) async fn evaluate_symbols<F, Fut>(
        &self,
        symbols: Vec<String>,
        mut trigger_ready: F,
    ) -> Result<(), JobError>
    where
        F: FnMut(String, HashSet<crate::domain::accounts::AccountId>) -> Fut,
        Fut: Future<Output = Result<Vec<TriggeredAlert>, AppError>>,
    {
        if !self.outbound.is_ready_for_alerts() {
            return Err(JobError::Unavailable(CHANNEL_RETRY));
        }
        let mut any_deferred = false;
        for symbol in symbols {
            let (ready_owners, symbol_deferred) = self.ready_owners_for_symbol(&symbol).await?;
            any_deferred |= symbol_deferred;
            if ready_owners.is_empty() {
                continue;
            }
            for hit in trigger_ready(symbol, ready_owners).await? {
                self.notify_hit(&hit).await;
            }
        }
        if any_deferred {
            return Err(JobError::Unavailable(CHANNEL_RETRY));
        }
        Ok(())
    }

    async fn notify_hit(&self, hit: &TriggeredAlert) {
        let owner = &hit.alert.owner_id;
        let identities = match self.services.identity.list(owner).await {
            Ok(ids) => ids,
            Err(err) => {
                warn!(%owner, error = %err, "Could not read linked identities for an alert");
                Vec::new()
            }
        };
        self.notify_identities(hit, identities).await;
    }

    /// Evaluates one symbol at a time and sends its DMs before the next
    /// symbol's price is fetched: a timeout or shutdown mid-check (with a
    /// single attempt) then loses at most the symbol in flight, not alerts
    /// already marked for earlier ones.
    #[cfg(test)]
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
                self.notify_hit(&hit).await;
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
    use crate::domain::channels::{Channel, PlatformIdentity, PlatformUserId};
    use crate::domain::jobs::JobId;
    use crate::domain::portfolio::alerts::AlertCondition;
    use crate::interface::channels::adapter::ChannelError;
    use crate::interface::jobs::fake::FakeOutbound;

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
        let handler = CheckAlertsHandler::new(s, FakeOutbound::disconnected());

        let out = handler.run(&ctx(None), serde_json::json!({})).await;

        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == CHANNEL_RETRY),
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
        let handler = CheckAlertsHandler::new(s.clone(), FakeOutbound::disconnected());

        let _ = handler.run(&ctx(None), serde_json::json!({})).await;

        assert_eq!(
            s.portfolio.list_active_alert_symbols().await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn check_alerts_with_no_alerts_sends_nothing() {
        let (s, _) = services(&["acct_pf_h_none"]).await;
        let fake = FakeOutbound::connected();
        let handler = CheckAlertsHandler::new(s, fake.clone());

        let out = handler.run(&ctx(None), serde_json::json!({})).await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.dms().is_empty());
    }

    /// Every linked identity gets the DM, and one failing send does not stop
    /// the rest.
    #[tokio::test]
    async fn a_triggered_alert_is_sent_to_every_linked_mxid() {
        let (s, _) = services(&["acct_pf_h_dm"]).await;
        let fake = FakeOutbound::connected();
        fake.results
            .lock()
            .unwrap()
            .push_back(Err(ChannelError::Transient("homeserver down".into())));
        let handler = CheckAlertsHandler::new(s, fake.clone());
        let hit = TriggeredAlert {
            alert: alert("above", 20_000),
            price_minor: 21_000,
        };

        handler
            .notify(&hit, vec!["@a:test".into(), "@b:test".into()])
            .await;

        let dms = fake.dms();
        let to: Vec<&str> = dms.iter().map(|d| d.0.as_str()).collect();
        assert_eq!(to, ["@a:test", "@b:test"]);
        assert!(dms[0].1.contains("Price Alert: AAPL"));
    }

    #[tokio::test]
    async fn a_triggered_alert_is_sent_to_non_matrix_linked_identities() {
        let (s, o) = services(&["acct_pf_h_tg_dm"]).await;
        let tg = PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, "555666").unwrap(),
            Some("Trader".into()),
        );
        let email = PlatformIdentity::new(
            Channel::Email,
            PlatformUserId::parse(Channel::Email, "trader@example.com").unwrap(),
            None,
        );
        s.identity.bind_for_test(&o[0], &tg).await;
        s.identity.bind_for_test(&o[0], &email).await;

        let fake = FakeOutbound::connected();
        let handler = CheckAlertsHandler::new(s, fake.clone());
        let owner = o[0];

        let out = handler
            .check_symbols(vec!["AAPL".into()], move |_| {
                let mut hit = alert("above", 20_000);
                hit.owner_id = owner;
                async move {
                    Ok(vec![TriggeredAlert {
                        alert: hit,
                        price_minor: 21_000,
                    }])
                }
            })
            .await;

        assert!(out.is_ok(), "{out:?}");
        let dms = fake.dms();
        let mut to: Vec<&str> = dms.iter().map(|d| d.0.as_str()).collect();
        to.sort_unstable();
        assert_eq!(to, ["555666", "trader@example.com"]);
    }

    /// The DMs for symbol A are out before symbol B is even evaluated, so a
    /// timeout or shutdown during B cannot lose A's already-marked alerts.
    #[tokio::test]
    async fn dms_for_one_symbol_are_sent_before_the_next_is_evaluated() {
        let (s, _) = services(&["acct_pf_h_order"]).await;
        let owner = s.identity.provision_matrix_for_test("@order:test").await;
        let fake = FakeOutbound::connected();
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
                        *seen_at_b.lock().unwrap() = Some(fake.dms().len());
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

    /// The scheduled refresh records today's snapshot, and a retry of the
    /// same job does not record a second one.
    #[tokio::test]
    async fn refresh_prices_records_one_snapshot_per_day() {
        use crate::infrastructure::postgres::repositories::portfolio::history as snapshots;
        let (s, o) = services(&["acct_pf_h_snapshot"]).await;
        let handler = RefreshPricesHandler::new(s.clone());

        // No holdings, so no price is fetched and no network is touched.
        handler
            .run(&ctx(Some(o[0])), serde_json::json!({}))
            .await
            .unwrap();
        handler
            .run(&ctx(Some(o[0])), serde_json::json!({}))
            .await
            .unwrap();

        // A cutoff two days back keeps the assertion stable across midnight.
        let cutoff = (chrono::Utc::now() - chrono::Duration::days(2))
            .format("%Y-%m-%d")
            .to_string();
        let rows = snapshots::get_snapshots(s.portfolio.pool(), &o[0], "USD", &cutoff)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "one snapshot per day, even after a retry");
    }

    #[tokio::test]
    async fn refresh_prices_without_an_account_is_permanent() {
        let (s, _) = services(&["acct_pf_h_noacct"]).await;
        let handler = RefreshPricesHandler::new(s);

        let out = handler.run(&ctx(None), serde_json::json!({})).await;

        assert!(matches!(out, Err(JobError::Permanent(_))), "{out:?}");
    }

    #[tokio::test]
    async fn matrix_only_owner_waits_when_matrix_is_down_and_telegram_is_up() {
        let (s, o) = services(&["acct_pf_mx_only", "acct_pf_tg_ready"]).await;
        let mx_owner = o[0];
        let tg_owner = o[1];

        let mx_id = PlatformIdentity::matrix("@mx_only:test").unwrap();
        let tg_id = PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, "888999").unwrap(),
            Some("TgUser".into()),
        );
        s.identity.bind_for_test(&mx_owner, &mx_id).await;
        s.identity.bind_for_test(&tg_owner, &tg_id).await;

        let mx_alert = s
            .portfolio
            .create_alert(&mx_owner, "AAPL", 20_000, "USD", 2, AlertCondition::Above)
            .await
            .unwrap();
        let tg_alert = s
            .portfolio
            .create_alert(&tg_owner, "AAPL", 20_000, "USD", 2, AlertCondition::Above)
            .await
            .unwrap();

        let fake = FakeOutbound::with_states([(Channel::Matrix, false), (Channel::Telegram, true)]);
        let handler = CheckAlertsHandler::new(s.clone(), fake.clone());
        let portfolio = s.portfolio.clone();

        let out = handler
            .evaluate_symbols(vec!["AAPL".into()], move |symbol, ready_owners| {
                let portfolio = portfolio.clone();
                async move {
                    let mut hits = Vec::new();
                    portfolio
                        .trigger_alerts_for_quote_for_owners(
                            &symbol,
                            "USD",
                            210.0,
                            Some(&ready_owners),
                            &mut hits,
                        )
                        .await?;
                    Ok(hits)
                }
            })
            .await;

        // The job defers because the Matrix-only owner's channel is down.
        assert!(
            matches!(out, Err(JobError::Unavailable(d)) if d == CHANNEL_RETRY),
            "{out:?}"
        );
        // The Telegram owner's alert was evaluated, marked triggered, and DM'd.
        let dms = fake.dms();
        assert_eq!(dms.len(), 1);
        assert_eq!(dms[0].0, "888999");
        assert!(
            s.portfolio
                .find_alert(&tg_owner, tg_alert.id)
                .await
                .unwrap()
                .unwrap()
                .is_triggered,
            "Telegram owner's alert must be marked triggered"
        );
        // The Matrix-only owner's alert must NOT be marked triggered while Matrix is down.
        assert!(
            !s.portfolio
                .find_alert(&mx_owner, mx_alert.id)
                .await
                .unwrap()
                .unwrap()
                .is_triggered,
            "Matrix-only owner's alert must remain active until Matrix reconnects"
        );
    }

    #[tokio::test]
    async fn telegram_linked_owner_on_deployment_without_telegram_falls_back_to_web() {
        let (s, o) = services(&["acct_pf_unconf_tg"]).await;
        let owner = o[0];

        let tg_id = PlatformIdentity::new(
            Channel::Telegram,
            PlatformUserId::parse(Channel::Telegram, "777888").unwrap(),
            Some("OldTg".into()),
        );
        s.identity.bind_for_test(&owner, &tg_id).await;

        let created = s
            .portfolio
            .create_alert(&owner, "AAPL", 20_000, "USD", 2, AlertCondition::Above)
            .await
            .unwrap();

        // Deployment has only Matrix configured (Telegram is disabled).
        let fake = FakeOutbound::with_configured([Channel::Matrix]);
        let handler = CheckAlertsHandler::new(s.clone(), fake.clone());
        let portfolio = s.portfolio.clone();

        let out = handler
            .evaluate_symbols(vec!["AAPL".into()], move |symbol, ready_owners| {
                let portfolio = portfolio.clone();
                async move {
                    let mut hits = Vec::new();
                    portfolio
                        .trigger_alerts_for_quote_for_owners(
                            &symbol,
                            "USD",
                            210.0,
                            Some(&ready_owners),
                            &mut hits,
                        )
                        .await?;
                    Ok(hits)
                }
            })
            .await;

        assert!(out.is_ok(), "{out:?}");
        assert!(fake.dms().is_empty());
        assert!(
            s.portfolio
                .find_alert(&owner, created.id)
                .await
                .unwrap()
                .unwrap()
                .is_triggered,
            "Alert for owner with only unconfigured channel identity falls back to web and triggers"
        );
        assert!(
            crate::infrastructure::postgres::repositories::outbox::event_types_for(
                s.portfolio.pool(),
                &owner
            )
            .await
            .unwrap()
            .contains(&"PortfolioAlertTriggered".to_string()),
            "PortfolioAlertTriggered published to outbox for web delivery"
        );
    }
}
