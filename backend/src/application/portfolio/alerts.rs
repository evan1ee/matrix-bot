use tracing::{debug, info, warn};

use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::portfolio::alerts::{AlertCondition, PriceAlert};
use crate::infrastructure::postgres::repositories::outbox;
use crate::infrastructure::postgres::repositories::portfolio::alerts as repo;
use crate::infrastructure::web::portfolio_feed::{fetch_latest_price, price_to_minor};

/// An alert that was just marked triggered, with the price that met it.
#[derive(Debug, Clone)]
pub struct TriggeredAlert {
    pub alert: PriceAlert,
    pub price_minor: i64,
}

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
        let mut tx = self.pool.begin().await?;
        let alert = repo::create(
            &mut *tx,
            owner,
            symbol,
            target_price_minor,
            currency,
            currency_exponent,
            condition,
        )
        .await?;

        outbox::publish(&mut tx, &SystemEvent::PortfolioAlertCreated(alert.clone())).await?;
        tx.commit().await?;
        Ok(alert)
    }

    pub async fn find_alert(
        &self,
        owner: &AccountId,
        id: i64,
    ) -> Result<Option<PriceAlert>, AppError> {
        let alert = repo::find_by_id(&self.pool, id, owner).await?;
        Ok(alert)
    }

    pub async fn list_alerts(&self, owner: &AccountId) -> Result<Vec<PriceAlert>, AppError> {
        let list = repo::list_for_owner(&self.pool, owner).await?;
        Ok(list)
    }

    pub async fn delete_alert(&self, owner: &AccountId, id: i64) -> Result<bool, AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repo::delete(&mut *tx, id, owner).await?;
        if deleted {
            outbox::publish(
                &mut tx,
                &SystemEvent::PortfolioAlertDeleted {
                    id,
                    owner_id: *owner,
                },
            )
            .await?;
        }
        tx.commit().await?;
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
        let mut tx = self.pool.begin().await?;
        let marked = repo::mark_triggered(&mut *tx, alert.id).await?;
        if marked {
            outbox::publish(
                &mut tx,
                &SystemEvent::PortfolioAlertTriggered(alert.clone()),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(marked)
    }

    /// Fetches `symbol`'s price, marks every met active alert triggered
    /// (publishing `PortfolioAlertTriggered`) and returns them. No active
    /// alert means no fetch; an unfetchable price is skipped.
    ///
    /// Marking is the idempotency guard and the caller notifies afterwards,
    /// so an alert marked here must reach the caller: a database error after
    /// some alerts were marked returns those (the rest are still active and
    /// the next check finds them) instead of losing their notifications.
    pub async fn trigger_symbol_alerts(
        &self,
        symbol: &str,
    ) -> Result<Vec<TriggeredAlert>, AppError> {
        if self.list_active_alerts_for_symbol(symbol).await?.is_empty() {
            return Ok(Vec::new());
        }
        let quote = match fetch_latest_price(symbol).await {
            Ok(q) => q,
            Err(err) => {
                debug!(%symbol, %err, "No price for alert check");
                return Ok(Vec::new());
            }
        };
        let mut triggered = Vec::new();
        match self
            .trigger_alerts_for_symbol(symbol, quote.price, &mut triggered)
            .await
        {
            Err(err) if triggered.is_empty() => Err(err),
            Err(err) => {
                warn!(%symbol, error = %err, "Alert check stopped early; notifying what triggered");
                Ok(triggered)
            }
            Ok(()) => Ok(triggered),
        }
    }

    /// Marks the active alerts on `symbol` that `price` meets and appends
    /// each to `triggered` as soon as it is marked, so a later failure cannot
    /// hide it from the caller.
    pub(crate) async fn trigger_alerts_for_symbol(
        &self,
        symbol: &str,
        price: f64,
        triggered: &mut Vec<TriggeredAlert>,
    ) -> Result<(), AppError> {
        for alert in self.list_active_alerts_for_symbol(symbol).await? {
            let price_minor = price_to_minor(price, alert.currency_exponent);
            if !alert
                .parsed_condition()
                .is_met(price_minor, alert.target_price_minor)
            {
                continue;
            }
            info!(alert_id = alert.id, %symbol, owner = %alert.owner_id, "Price alert triggered");
            if self.mark_alert_triggered(&alert).await? {
                triggered.push(TriggeredAlert { alert, price_minor });
            }
        }
        Ok(())
    }
}
