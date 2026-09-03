use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::modules::portfolio::price_feed;
use crate::modules::ModuleContext;

/// Interval between automatic portfolio price refresh sweeps (1 hour).
pub const PRICE_SWEEP_INTERVAL: Duration = Duration::from_secs(3600);

/// Starts the background task that periodically fetches latest market prices
/// for all active holdings across all users.
pub fn start_portfolio_price_scheduler(ctx: ModuleContext) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Starting Portfolio Market Price Scheduler");

        loop {
            tokio::time::sleep(PRICE_SWEEP_INTERVAL).await;
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
                    continue;
                }
            };

            for owner in owners {
                let summary = price_feed::refresh_all_for_owner(
                    &ctx.db.pool,
                    ctx.broadcaster.as_ref(),
                    &owner,
                )
                .await;

                if !summary.updated.is_empty() {
                    debug!(
                        "Scheduled portfolio sweep updated {} holdings for {}",
                        summary.updated.len(),
                        owner
                    );
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sweep_interval() {
        assert_eq!(PRICE_SWEEP_INTERVAL.as_secs(), 3600);
    }
}
