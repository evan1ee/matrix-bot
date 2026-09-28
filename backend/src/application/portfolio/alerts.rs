use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::alerts::{AlertCondition, PriceAlert};
use crate::events::SystemEvent;
use crate::infrastructure::postgres::repositories::portfolio::alerts as repo;

impl PortfolioService {
    pub async fn create_alert(
        &self,
        owner: &AccountId,
        symbol: &str,
        target_price_minor: i64,
        currency: &str,
        currency_exponent: i64,
        condition: AlertCondition,
    ) -> Result<PriceAlert, AppError> {
        let alert = repo::create(
            &self.pool,
            owner.as_str(),
            symbol,
            target_price_minor,
            currency,
            currency_exponent,
            condition,
        )
        .await?;

        self.events
            .send(SystemEvent::PortfolioAlertCreated(alert.clone()));
        Ok(alert)
    }

    pub async fn find_alert(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Option<PriceAlert>, AppError> {
        let alert = repo::find_by_id(&self.pool, id, owner.as_str()).await?;
        Ok(alert)
    }

    pub async fn list_alerts(&self, owner: &AccountId) -> Result<Vec<PriceAlert>, AppError> {
        let list = repo::list_for_owner(&self.pool, owner.as_str()).await?;
        Ok(list)
    }

    pub async fn delete_alert(&self, owner: &AccountId, id: i64) -> Result<bool, AppError> {
        let deleted = repo::delete(&self.pool, id, owner.as_str()).await?;
        if deleted {
            self.events.send(SystemEvent::PortfolioAlertDeleted {
                id,
                owner_id: owner.to_string(),
            });
        }
        Ok(deleted)
    }

    pub async fn list_active_alert_symbols(&self) -> Result<Vec<String>, AppError> {
        let symbols = repo::list_active_symbols(&self.pool).await?;
        Ok(symbols)
    }

    pub async fn list_active_alerts_for_symbol(
        &self,
        symbol: &str,
    ) -> Result<Vec<PriceAlert>, AppError> {
        let alerts = repo::list_active_for_symbol(&self.pool, symbol).await?;
        Ok(alerts)
    }

    pub async fn mark_alert_triggered(&self, alert: &PriceAlert) -> Result<bool, AppError> {
        let marked = repo::mark_triggered(&self.pool, alert.id).await?;
        if marked {
            self.events
                .send(SystemEvent::PortfolioAlertTriggered(alert.clone()));
        }
        Ok(marked)
    }
}
