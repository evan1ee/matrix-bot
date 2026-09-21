//! Cash balance persistence and atomic credit/debit operations.

use crate::db::DbPool;
use anyhow::Result;

use crate::modules::portfolio::models::{CashBalance, WithdrawOutcome};

pub const CASH_COLUMNS: &str =
    "id, owner_id, currency, currency_exponent, balance_minor, updated_at";

pub async fn get_cash_or_zero(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
) -> Result<CashBalance> {
    let found = crate::db_fetch_optional!(
        pool,
        CashBalance,
        &format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?"
    ),
        owner_id,
        currency
    )?;

    Ok(found.unwrap_or(CashBalance {
        id: 0,
        owner_id: owner_id.to_string(),
        currency: currency.to_string(),
        currency_exponent,
        balance_minor: 0,
        updated_at: None,
    }))
}

pub async fn list_cash(pool: &DbPool, owner_id: &str) -> Result<Vec<CashBalance>> {
    let balances = crate::db_fetch_all!(pool, CashBalance, &format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? ORDER BY currency ASC"
    ), owner_id)?;

    Ok(balances)
}

pub async fn credit_cash(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
) -> Result<CashBalance> {
    crate::db_execute!(pool, "INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at) \
         VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(owner_id, currency) DO UPDATE SET \
            balance_minor = balance_minor + excluded.balance_minor, \
            updated_at = CURRENT_TIMESTAMP", owner_id, currency, currency_exponent, amount_minor)?;

    get_cash_or_zero(pool, owner_id, currency, currency_exponent).await
}

pub async fn deposit_cash(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
) -> Result<CashBalance> {
    deposit_cash_at(
        pool,
        owner_id,
        currency,
        currency_exponent,
        amount_minor,
        None,
    )
    .await
}

pub async fn deposit_cash_at(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<CashBalance> {
    let cash = credit_cash(pool, owner_id, currency, currency_exponent, amount_minor).await?;
    let _ = crate::modules::portfolio::audit::record_audit_entry(
        pool,
        owner_id,
        "deposit",
        None,
        None,
        None,
        amount_minor,
        currency,
        currency_exponent,
        None,
        None,
        executed_at,
    )
    .await;
    Ok(cash)
}

pub async fn debit_cash(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    amount_minor: i64,
) -> Result<WithdrawOutcome> {
    let existing: Option<(i64, i64)> = crate::db_fetch_optional!(
        pool,
        _,
        "SELECT id, balance_minor FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?",
        owner_id,
        currency
    )?;

    let (id, balance) = match existing {
        Some(row) => row,
        None => return Ok(WithdrawOutcome::InsufficientFunds { available_minor: 0 }),
    };

    if amount_minor > balance {
        return Ok(WithdrawOutcome::InsufficientFunds {
            available_minor: balance,
        });
    }

    crate::db_execute!(pool, "UPDATE portfolio_cash_balances SET balance_minor = balance_minor - ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", amount_minor, id)?;

    let updated = crate::db_fetch_one!(
        pool,
        CashBalance,
        &format!("SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE id = ?"),
        id
    )?;

    Ok(WithdrawOutcome::Ok(updated))
}

pub async fn withdraw_cash(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    amount_minor: i64,
) -> Result<WithdrawOutcome> {
    withdraw_cash_at(pool, owner_id, currency, amount_minor, None).await
}

pub async fn withdraw_cash_at(
    pool: &DbPool,
    owner_id: &str,
    currency: &str,
    amount_minor: i64,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<WithdrawOutcome> {
    let outcome = debit_cash(pool, owner_id, currency, amount_minor).await?;
    if let WithdrawOutcome::Ok(ref cash) = outcome {
        let _ = crate::modules::portfolio::audit::record_audit_entry(
            pool,
            owner_id,
            "withdraw",
            None,
            None,
            None,
            amount_minor,
            currency,
            cash.currency_exponent,
            None,
            None,
            executed_at,
        )
        .await;
    }
    Ok(outcome)
}
