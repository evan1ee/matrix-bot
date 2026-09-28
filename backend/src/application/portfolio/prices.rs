use tracing::{error, info, warn};

use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::models::Holding;
use crate::events::SystemEvent;
use crate::infrastructure::postgres::repositories::portfolio::holdings as repo;
use crate::infrastructure::web::portfolio_feed::{
    fetch_latest_price, price_to_minor, search_symbols, TickerSearchResult,
};

#[derive(Debug, Default, Clone)]
pub struct RefreshSummary {
    pub updated: Vec<String>,
    pub failed: Vec<String>,
}

impl PortfolioService {
    pub async fn refresh_holding_price(
        &self,
        owner: &AccountId,
        holding: &Holding,
    ) -> Result<Holding, AppError> {
        let fetched =
            fetch_latest_price(&holding.symbol)
                .await
                .map_err(|e| AppError::External {
                    service: "portfolio_price_feed",
                    source: anyhow::anyhow!(e),
                })?;

        let price_minor = price_to_minor(fetched.price, holding.currency_exponent);

        let updated = repo::set_price(&self.pool, owner.as_str(), &holding.symbol, price_minor)
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("Holding for {} not found", holding.symbol))
            })?;

        self.events
            .send(SystemEvent::PortfolioHoldingUpdated(updated.clone()));

        info!(
            "Refreshed price for {} (owner {}): {:.2} {} ({} minor units)",
            holding.symbol,
            owner.as_str(),
            fetched.price,
            holding.currency,
            price_minor
        );

        Ok(updated)
    }

    pub async fn refresh_all_for_owner(&self, owner: &AccountId) -> RefreshSummary {
        let mut summary = RefreshSummary::default();

        let holdings = match repo::list(&self.pool, owner.as_str()).await {
            Ok(h) => h,
            Err(e) => {
                error!(
                    "Failed to list holdings for owner {}: {}",
                    owner.as_str(),
                    e
                );
                return summary;
            }
        };

        for holding in holdings {
            match self.refresh_holding_price(owner, &holding).await {
                Ok(_) => summary.updated.push(holding.symbol),
                Err(err) => {
                    warn!("Failed to refresh price for {}: {}", holding.symbol, err);
                    summary.failed.push(holding.symbol);
                }
            }
        }

        summary
    }

    pub async fn search_symbols(&self, query: &str) -> Result<Vec<TickerSearchResult>, AppError> {
        let results = search_symbols(query)
            .await
            .map_err(|e| AppError::External {
                service: "portfolio_symbol_search",
                source: anyhow::anyhow!(e),
            })?;
        Ok(results)
    }
}
