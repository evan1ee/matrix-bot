use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::accounts::AccountRepository;
use crate::bot::notify::send_dm_html;
use crate::events::SystemEvent;
use crate::modules::portfolio::alerts::{format_alert_notification, AlertCondition, AlertsRepository};
use crate::modules::portfolio::history::HistoryRepository;
use crate::modules::portfolio::price_feed;
use crate::modules::ModuleContext;

/// Interval between automatic portfolio price refresh sweeps (1 hour).
pub const PRICE_SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// Interval between checking active price alerts (2 minutes).
pub const ALERT_CHECK_INTERVAL: Duration = Duration::from_secs(120);

/// Checks all active alerts against latest market prices and notifies users on Matrix.
pub async fn check_price_alerts(ctx: &ModuleContext) {
    let active_symbols = match AlertsRepository::list_active_symbols(&ctx.db.pool).await {
        Ok(s) if !s.is_empty() => s,
        _ => return, // No active alerts in the system
    };

    debug!("Evaluating active price alerts for {} symbols", active_symbols.len());

    for sym in active_symbols {
        let quote = match price_feed::fetch_latest_price(&sym).await {
            Ok(q) => q,
            Err(err) => {
                debug!("Could not fetch price for alert check on {}: {}", sym, err);
                continue;
            }
        };

        let alerts = match AlertsRepository::list_active_for_symbol(&ctx.db.pool, &sym).await {
            Ok(list) => list,
            Err(_) => continue,
        };

        for alert in alerts {
            let price_minor = price_feed::price_to_minor(quote.price, alert.currency_exponent as i64);
            let condition_met = match alert.parsed_condition() {
                AlertCondition::Above => price_minor >= alert.target_price_minor,
                AlertCondition::Below => price_minor <= alert.target_price_minor,
            };

            if condition_met {
                info!(
                    "Triggering price alert #{} on {} for owner {} (target: {}, actual: {})",
                    alert.id, alert.symbol, alert.owner_id, alert.target_price_minor, price_minor
                );

                if let Ok(true) = AlertsRepository::mark_triggered(&ctx.db.pool, alert.id).await {
                    if let Some(b) = ctx.broadcaster.as_ref() {
                        b.send(SystemEvent::PortfolioAlertTriggered(alert.clone()));
                    }

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

                let owners = match sqlx::query_scalar::<_, String>(
                    "SELECT DISTINCT owner_id FROM portfolio_holdings;"
                )
                .fetch_all(&ctx.db.pool)
                .await
                {
                    Ok(list) => list,
                    Err(err) => {
                        warn!("Failed querying owners for portfolio price sweep: {}", err);
                        Vec::new()
                    }
                };

                for owner in &owners {
                    let summary = price_feed::refresh_all_for_owner(
                        &ctx.db.pool,
                        ctx.broadcaster.as_ref(),
                        owner,
                    )
                    .await;

                    if !summary.updated.is_empty() {
                        debug!(
                            "Scheduled portfolio sweep updated {} holdings for {}",
                            summary.updated.len(),
                            owner
                        );
                    }

                    // Record daily snapshot for performance charts
                    let _ = HistoryRepository::capture_current_snapshot_for_owner(&ctx.db.pool, owner).await;
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
