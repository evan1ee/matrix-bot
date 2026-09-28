use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::models::{Holding, PortfolioCurrencySummary};
use crate::events::SystemEvent;
use crate::infrastructure::postgres::repositories::portfolio::holdings as repo;

impl PortfolioService {
    pub async fn list_holdings(&self, owner: &AccountId) -> Result<Vec<Holding>, AppError> {
        let list = repo::list(&self.pool, owner.as_str()).await?;
        Ok(list)
    }

    pub async fn get_holding_by_symbol(
        &self,
        owner: &AccountId,
        symbol: &str,
    ) -> Result<Option<Holding>, AppError> {
        let holding = repo::find(&self.pool, owner.as_str(), symbol).await?;
        Ok(holding)
    }

    pub async fn get_holding_owned(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Option<Holding>, AppError> {
        let holding = repo::find_owned(&self.pool, id, owner.as_str()).await?;
        Ok(holding)
    }

    pub async fn set_price(
        &self,
        owner: &AccountId,
        symbol: &str,
        price_minor: i64,
    ) -> Result<Option<Holding>, AppError> {
        let updated = repo::set_price(&self.pool, owner.as_str(), symbol, price_minor).await?;
        if let Some(ref h) = updated {
            self.events
                .send(SystemEvent::PortfolioHoldingUpdated(h.clone()));
        }
        Ok(updated)
    }

    pub async fn set_note(
        &self,
        owner: &AccountId,
        symbol: &str,
        note: Option<&str>,
    ) -> Result<Option<Holding>, AppError> {
        let updated = repo::set_note(&self.pool, owner.as_str(), symbol, note).await?;
        if let Some(ref h) = updated {
            self.events
                .send(SystemEvent::PortfolioHoldingUpdated(h.clone()));
        }
        Ok(updated)
    }

    pub async fn update_symbol(
        &self,
        owner: &AccountId,
        id: i64,
        new_symbol: &str,
    ) -> Result<Result<Holding, String>, AppError> {
        let res = repo::update_symbol(&self.pool, owner.as_str(), id, new_symbol).await?;
        if let Ok(ref h) = res {
            self.events
                .send(SystemEvent::PortfolioHoldingUpdated(h.clone()));
        }
        Ok(res)
    }

    pub async fn delete_holding(&self, owner: &AccountId, symbol: &str) -> Result<bool, AppError> {
        let existing = self.get_holding_by_symbol(owner, symbol).await?;
        let deleted = repo::delete(&self.pool, owner.as_str(), symbol).await?;
        if deleted {
            if let Some(h) = existing {
                self.events.send(SystemEvent::PortfolioHoldingDeleted {
                    id: h.id,
                    owner_id: owner.to_string(),
                });
            }
        }
        Ok(deleted)
    }

    pub async fn summary(
        &self,
        owner: &AccountId,
    ) -> Result<Vec<PortfolioCurrencySummary>, AppError> {
        let res = repo::summary(&self.pool, owner.as_str()).await?;
        Ok(res)
    }

    pub async fn list_holding_owners(&self) -> Result<Vec<String>, AppError> {
        let owners = repo::list_holding_owners(&self.pool).await?;
        Ok(owners)
    }
}
