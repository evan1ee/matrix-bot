use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info};

use crate::accounts::AccountRepository;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::alerts::{AlertCondition, PriceAlert};
use crate::infrastructure::web::portfolio_feed::{fetch_latest_price, price_to_minor};
use crate::interface::channels::matrix::notify::send_dm_html;
use crate::interface::chat::ModuleContext;
use crate::utils::html::html_escape;

/// Interval between automatic portfolio price refresh sweeps (1 hour).
pub const PRICE_SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// Interval between checking active price alerts (2 minutes).
pub const ALERT_CHECK_INTERVAL: Duration = Duration::from_secs(120);

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

/// Checks all active alerts against latest market prices and notifies users on Matrix.
pub async fn check_price_alerts(ctx: &ModuleContext) {
    let active_symbols = match ctx.services.portfolio.list_active_alert_symbols().await {
        Ok(s) if !s.is_empty() => s,
        _ => return, // No active alerts in the system
    };

    debug!(
        "Evaluating active price alerts for {} symbols",
        active_symbols.len()
    );

    for sym in active_symbols {
        let quote = match fetch_latest_price(&sym).await {
            Ok(q) => q,
            Err(err) => {
                debug!("Could not fetch price for alert check on {}: {}", sym, err);
                continue;
            }
        };

        let alerts = match ctx
            .services
            .portfolio
            .list_active_alerts_for_symbol(&sym)
            .await
        {
            Ok(list) => list,
            Err(_) => continue,
        };

        for alert in alerts {
            let price_minor = price_to_minor(quote.price, alert.currency_exponent);
            let condition_met = match alert.parsed_condition() {
                AlertCondition::Above => price_minor >= alert.target_price_minor,
                AlertCondition::Below => price_minor <= alert.target_price_minor,
            };

            if condition_met {
                info!(
                    "Triggering price alert #{} on {} for owner {} (target: {}, actual: {})",
                    alert.id, alert.symbol, alert.owner_id, alert.target_price_minor, price_minor
                );

                if let Ok(true) = ctx.services.portfolio.mark_alert_triggered(&alert).await {
                    let (plain, html) = format_alert_notification(&alert, price_minor);
                    let mxids = AccountRepository::linked_mxids(&ctx.db.pool, &alert.owner_id)
                        .await
                        .unwrap_or_default();

                    for mxid in mxids {
                        send_dm_html(&ctx.client, &mxid, &plain, &html).await;
                    }
                }
            }
        }
    }
}

/// Starts the background task that periodically fetches latest market prices,
/// checks price alerts, and records daily portfolio snapshots.
pub fn start_portfolio_price_scheduler(ctx: ModuleContext) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting Portfolio Market Price & Alert Scheduler");

        let mut last_sweep = tokio::time::Instant::now() - PRICE_SWEEP_INTERVAL;

        loop {
            // Check price alerts frequently (every 2 minutes)
            check_price_alerts(&ctx).await;

            // Run full portfolio price sweep and snapshot capture hourly
            if last_sweep.elapsed() >= PRICE_SWEEP_INTERVAL {
                last_sweep = tokio::time::Instant::now();
                debug!("Running scheduled portfolio price refresh sweep");

                let owners = ctx
                    .services
                    .portfolio
                    .list_holding_owners()
                    .await
                    .unwrap_or_default();

                for owner in &owners {
                    let acct = AccountId::new(owner);
                    let summary = ctx.services.portfolio.refresh_all_for_owner(&acct).await;

                    if !summary.updated.is_empty() {
                        debug!(
                            "Scheduled portfolio sweep updated {} holdings for {}",
                            summary.updated.len(),
                            owner
                        );
                    }

                    // Record daily snapshot for performance charts
                    let _ = ctx
                        .services
                        .portfolio
                        .capture_current_snapshot_for_owner(&acct)
                        .await;
                }
            }

            tokio::time::sleep(ALERT_CHECK_INTERVAL).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sweep_interval() {
        assert_eq!(PRICE_SWEEP_INTERVAL.as_secs(), 3600);
        assert_eq!(ALERT_CHECK_INTERVAL.as_secs(), 120);
    }
}
