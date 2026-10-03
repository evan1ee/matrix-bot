use chrono::{DateTime, Utc};

use super::cash::cash_event;
use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::domain::portfolio::models::{
    normalise_symbol, BuyOutcome, Holding, SellOutcome, QUANTITY_EXPONENT,
};
use crate::domain::portfolio::{validate_note, PortfolioError, TradeRequest};
use crate::infrastructure::postgres::repositories::outbox;
use crate::infrastructure::postgres::repositories::portfolio::holdings as holdings_repo;
use crate::infrastructure::postgres::repositories::portfolio::trades as trades_repo;

struct ParsedTrade {
    symbol: String,
    currency: String,
    exponent: u32,
    price_minor: i64,
    quantity_minor: i64,
}

fn parse_trade(req: &TradeRequest<'_>) -> Result<ParsedTrade, PortfolioError> {
    let symbol = normalise_symbol(req.symbol).map_err(PortfolioError::Invalid)?;
    let currency =
        normalise_currency(req.currency).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let price_minor =
        parse_amount(req.price, exponent).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let quantity_minor = parse_amount(req.quantity, QUANTITY_EXPONENT)
        .map_err(|e| PortfolioError::Invalid(e.to_string()))?;

    Ok(ParsedTrade {
        symbol,
        currency,
        exponent,
        price_minor,
        quantity_minor,
    })
}

impl PortfolioService {
    pub async fn buy(
        &self,
        owner: &AccountId,
        req: TradeRequest<'_>,
    ) -> Result<BuyOutcome, AppError> {
        let note = validate_note(req.note)?;
        let parsed = parse_trade(&req)?;

        let mut tx = self.pool.begin().await?;
        let already_held = holdings_repo::find(&mut *tx, owner, &parsed.symbol)
            .await?
            .is_some();

        let outcome = trades_repo::buy_at(
            &mut tx,
            owner,
            &parsed.symbol,
            parsed.quantity_minor,
            parsed.price_minor,
            &parsed.currency,
            parsed.exponent as i64,
            note,
            req.executed_at,
        )
        .await?;

        if let BuyOutcome::Ok {
            ref holding,
            ref cash,
        } = outcome
        {
            let holding_event = if already_held {
                SystemEvent::PortfolioHoldingUpdated(Holding::clone(holding))
            } else {
                SystemEvent::PortfolioHoldingCreated(Holding::clone(holding))
            };
            outbox::publish(&mut tx, &holding_event).await?;
            outbox::publish(&mut tx, &cash_event(owner, cash)).await?;
        }
        tx.commit().await?;

        Ok(outcome)
    }

    pub async fn sell(
        &self,
        owner: &AccountId,
        symbol: &str,
        quantity: &str,
        price: &str,
        executed_at: Option<DateTime<Utc>>,
    ) -> Result<SellOutcome, AppError> {
        let symbol = normalise_symbol(symbol).map_err(PortfolioError::Invalid)?;

        let existing = holdings_repo::find(&self.pool, owner, &symbol).await?;
        let Some(existing) = existing else {
            return Ok(SellOutcome::NotFound);
        };
        let exponent = existing.currency_exponent as u32;

        let price_minor =
            parse_amount(price, exponent).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
        let quantity_minor = parse_amount(quantity, QUANTITY_EXPONENT)
            .map_err(|e| PortfolioError::Invalid(e.to_string()))?;

        let mut tx = self.pool.begin().await?;
        let outcome = trades_repo::sell_at(
            &mut tx,
            owner,
            &symbol,
            quantity_minor,
            price_minor,
            executed_at,
        )
        .await?;

        match &outcome {
            SellOutcome::Closed { cash, .. } => {
                let closed = SystemEvent::PortfolioHoldingDeleted {
                    id: existing.id,
                    owner_id: *owner,
                };
                outbox::publish(&mut tx, &closed).await?;
                outbox::publish(&mut tx, &cash_event(owner, cash)).await?;
            }
            SellOutcome::Reduced { holding, cash, .. } => {
                outbox::publish(
                    &mut tx,
                    &SystemEvent::PortfolioHoldingUpdated(holding.clone()),
                )
                .await?;
                outbox::publish(&mut tx, &cash_event(owner, cash)).await?;
            }
            SellOutcome::NotFound | SellOutcome::ExceedsHolding { .. } => {}
        }
        tx.commit().await?;

        Ok(outcome)
    }

    pub async fn record(
        &self,
        owner: &AccountId,
        req: TradeRequest<'_>,
    ) -> Result<Result<Holding, String>, AppError> {
        let note = validate_note(req.note)?;
        let parsed = parse_trade(&req)?;

        let mut tx = self.pool.begin().await?;
        let already_held = holdings_repo::find(&mut *tx, owner, &parsed.symbol)
            .await?
            .is_some();

        let res = trades_repo::record_holding_action(
            &mut tx,
            owner,
            &parsed.symbol,
            parsed.quantity_minor,
            parsed.price_minor,
            &parsed.currency,
            parsed.exponent as i64,
            note,
            "record",
            req.executed_at,
        )
        .await?;

        if let Ok(ref holding) = res {
            let event = if already_held {
                SystemEvent::PortfolioHoldingUpdated(holding.clone())
            } else {
                SystemEvent::PortfolioHoldingCreated(holding.clone())
            };
            outbox::publish(&mut tx, &event).await?;
        }
        tx.commit().await?;

        Ok(res)
    }
}
