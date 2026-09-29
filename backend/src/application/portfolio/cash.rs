use chrono::{DateTime, Utc};
use tracing::error;

use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::finance::{Direction, NewTransaction, Transaction, TransactionSource};
use crate::domain::portfolio::models::{CashBalance, WithdrawOutcome};
use crate::events::SystemEvent;
use crate::infrastructure::postgres::repositories::portfolio::cash as repo;

pub fn cash_event(owner_id: &AccountId, cash: &CashBalance) -> SystemEvent {
    SystemEvent::PortfolioCashUpdated {
        owner_id: owner_id.to_string(),
        currency: cash.currency.clone(),
        balance_minor: cash.balance_minor,
    }
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
        let cash = repo::deposit_cash_at(
            &self.pool,
            owner,
            currency,
            currency_exponent,
            amount_minor,
            executed_at,
        )
        .await?;

        self.events.send(cash_event(owner, &cash));

        self.record_portfolio_cash_transaction(
            owner,
            "deposit",
            amount_minor,
            currency,
            currency_exponent,
            cash.id,
            executed_at,
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
        let outcome =
            repo::withdraw_cash_at(&self.pool, owner, currency, amount_minor, executed_at).await?;

        if let WithdrawOutcome::Ok(ref cash) = outcome {
            self.events.send(cash_event(owner, cash));

            self.record_portfolio_cash_transaction(
                owner,
                "withdraw",
                amount_minor,
                currency,
                currency_exponent,
                cash.id,
                executed_at,
            )
            .await;
        }

        Ok(outcome)
    }

    pub async fn record_portfolio_cash_transaction(
        &self,
        owner: &AccountId,
        action: &str,
        amount_minor: i64,
        currency: &str,
        exponent: i64,
        cash_id: i64,
        executed_at: Option<DateTime<Utc>>,
    ) -> Option<Transaction> {
        let (direction, note) = match action {
            "deposit" => (Direction::Expense, "Portfolio cash deposit"),
            "withdraw" => (Direction::Income, "Portfolio cash withdrawal"),
            _ => return None,
        };

        let finance = crate::application::finance::FinanceService::new(
            self.pool.clone(),
            self.events.clone(),
        );

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
