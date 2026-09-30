use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::portfolio::models::{Holding, PortfolioCurrencySummary};
use crate::infrastructure::postgres::repositories::outbox;
use crate::infrastructure::postgres::repositories::portfolio::holdings as repo;

impl PortfolioService {
    pub async fn list_holdings(&self, owner: &AccountId) -> Result<Vec<Holding>, AppError> {
        let list = repo::list(&self.pool, owner).await?;
        Ok(list)
    }

    pub async fn get_holding_by_symbol(
        &self,
        owner: &AccountId,
        symbol: &str,
    ) -> Result<Option<Holding>, AppError> {
        let holding = repo::find(&self.pool, owner, symbol).await?;
        Ok(holding)
    }

    pub async fn get_holding_owned(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Option<Holding>, AppError> {
        let holding = repo::find_owned(&self.pool, id, owner).await?;
        Ok(holding)
    }

    pub async fn set_price(
        &self,
        owner: &AccountId,
        symbol: &str,
        price_minor: i64,
    ) -> Result<Option<Holding>, AppError> {
        let mut tx = self.pool.begin().await?;
        let updated = repo::set_price(&mut tx, owner, symbol, price_minor).await?;
        if let Some(ref h) = updated {
            outbox::publish(&mut tx, &SystemEvent::PortfolioHoldingUpdated(h.clone())).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn set_note(
        &self,
        owner: &AccountId,
        symbol: &str,
        note: Option<&str>,
    ) -> Result<Option<Holding>, AppError> {
        let mut tx = self.pool.begin().await?;
        let updated = repo::set_note(&mut tx, owner, symbol, note).await?;
        if let Some(ref h) = updated {
            outbox::publish(&mut tx, &SystemEvent::PortfolioHoldingUpdated(h.clone())).await?;
        }
        tx.commit().await?;
        Ok(updated)
    }

    pub async fn update_symbol(
        &self,
        owner: &AccountId,
        id: i64,
        new_symbol: &str,
    ) -> Result<Result<Holding, String>, AppError> {
        let mut tx = self.pool.begin().await?;
        let res = repo::update_symbol(&mut tx, owner, id, new_symbol).await?;
        if let Ok(ref h) = res {
            outbox::publish(&mut tx, &SystemEvent::PortfolioHoldingUpdated(h.clone())).await?;
        }
        tx.commit().await?;
        Ok(res)
    }

    pub async fn delete_holding(&self, owner: &AccountId, symbol: &str) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let existing = repo::find(&mut *tx, owner, symbol).await?;
        let deleted = repo::delete(&mut tx, owner, symbol).await?;
        if deleted {
            if let Some(h) = existing {
                outbox::publish(
                    &mut tx,
                    &SystemEvent::PortfolioHoldingDeleted {
                        id: h.id,
                        owner_id: *owner,
                    },
                )
                .await?;
            }
        }
        tx.commit().await?;
        Ok(deleted)
    }

    pub async fn summary(
        &self,
        owner: &AccountId,
    ) -> Result<Vec<PortfolioCurrencySummary>, AppError> {
        let res = repo::summary(&self.pool, owner).await?;
        Ok(res)
    }

    pub async fn list_holding_owners(&self) -> Result<Vec<AccountId>, AppError> {
        let owners = repo::list_holding_owners(&self.pool).await?;
        Ok(owners)
    }
}
