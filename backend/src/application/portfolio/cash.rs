use chrono::{DateTime, Utc};
use tracing::error;

use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::finance::{Direction, NewTransaction, Transaction, TransactionSource};
use crate::domain::portfolio::models::{CashBalance, WithdrawOutcome};
use crate::infrastructure::postgres::repositories::outbox;
use crate::infrastructure::postgres::repositories::portfolio::cash as repo;

pub fn cash_event(owner_id: &AccountId, cash: &CashBalance) -> SystemEvent {
    SystemEvent::PortfolioCashUpdated {
        owner_id: *owner_id,
        currency: cash.currency.clone(),
        balance_minor: cash.balance_minor,
    }
}

/// The ledger side of one cash movement, mirrored into the finance ledger.
struct CashMovement<'a> {
    amount_minor: i64,
    currency: &'a str,
    exponent: i64,
    cash_id: i64,
    executed_at: Option<DateTime<Utc>>,
}

impl PortfolioService {
    pub async fn get_cash(
        &self,
        owner: &AccountId,
        currency: &str,
        currency_exponent: i64,
    ) -> Result<CashBalance, AppError> {
        let cash = repo::get_cash_or_zero(&self.pool, owner, currency, currency_exponent).await?;
        Ok(cash)
    }

    pub async fn list_cash(&self, owner: &AccountId) -> Result<Vec<CashBalance>, AppError> {
        let list = repo::list_cash(&self.pool, owner).await?;
        Ok(list)
    }

    pub async fn deposit_cash(
        &self,
        owner: &AccountId,
        currency: &str,
        currency_exponent: i64,
        amount_minor: i64,
        executed_at: Option<DateTime<Utc>>,
    ) -> Result<CashBalance, AppError> {
        let mut tx = self.pool.begin().await?;
        let cash = repo::deposit_cash_at(
            &mut tx,
            owner,
            currency,
            currency_exponent,
            amount_minor,
            executed_at,
        )
        .await?;

        outbox::publish(&mut tx, &cash_event(owner, &cash)).await?;
        tx.commit().await?;

        self.record_portfolio_cash_transaction(
            owner,
            "deposit",
            CashMovement {
                amount_minor,
                currency,
                exponent: currency_exponent,
                cash_id: cash.id,
                executed_at,
            },
        )
        .await;

        Ok(cash)
    }

    pub async fn withdraw_cash(
        &self,
        owner: &AccountId,
        currency: &str,
        currency_exponent: i64,
        amount_minor: i64,
        executed_at: Option<DateTime<Utc>>,
    ) -> Result<WithdrawOutcome, AppError> {
        let mut tx = self.pool.begin().await?;
        let outcome =
            repo::withdraw_cash_at(&mut tx, owner, currency, amount_minor, executed_at).await?;

        if let WithdrawOutcome::Ok(ref cash) = outcome {
            outbox::publish(&mut tx, &cash_event(owner, cash)).await?;
        }
        tx.commit().await?;

        if let WithdrawOutcome::Ok(ref cash) = outcome {
            self.record_portfolio_cash_transaction(
                owner,
                "withdraw",
                CashMovement {
                    amount_minor,
                    currency,
                    exponent: currency_exponent,
                    cash_id: cash.id,
                    executed_at,
                },
            )
            .await;
        }

        Ok(outcome)
    }

    async fn record_portfolio_cash_transaction(
        &self,
        owner: &AccountId,
        action: &str,
        movement: CashMovement<'_>,
    ) -> Option<Transaction> {
        let CashMovement {
            amount_minor,
            currency,
            exponent,
            cash_id,
            executed_at,
        } = movement;
        let (direction, note) = match action {
            "deposit" => (Direction::Expense, "Portfolio cash deposit"),
            "withdraw" => (Direction::Income, "Portfolio cash withdrawal"),
            _ => return None,
        };

        let finance = crate::application::finance::FinanceService::new(self.pool.clone());

        finance
            .create_transaction(
                owner,
                NewTransaction {
                    direction,
                    amount_minor,
                    currency,
                    currency_exponent: exponent,
                    category: Some("Investments"),
                    note: Some(note),
                    room_id: "web",
                    sender_name: "Portfolio",
                    occurred_at: executed_at.unwrap_or_else(Utc::now),
                    source: Some((TransactionSource::PortfolioCash.as_str(), cash_id)),
                    create_missing_category: true,
                },
            )
            .await
            .map_err(|err| error!("Failed to record portfolio cash transaction: {:?}", err))
            .ok()
    }
}
