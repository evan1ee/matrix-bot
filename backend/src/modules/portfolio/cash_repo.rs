//! Cash balance persistence and atomic credit/debit operations.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::modules::portfolio::models::{CashBalance, WithdrawOutcome};

pub const CASH_COLUMNS: &str =
    "id, owner_id, currency, currency_exponent, balance_minor, updated_at";

pub async fn get_cash_or_zero(
    pool: &SqlitePool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
) -> Result<CashBalance> {
    let found = sqlx::query_as::<_, CashBalance>(&format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?"
    ))
    .bind(owner_id)
    .bind(currency)
    .fetch_optional(pool)
    .await?;

    Ok(found.unwrap_or(CashBalance {
        id: 0,
        owner_id: owner_id.to_string(),
        currency: currency.to_string(),
        currency_exponent,
        balance_minor: 0,
        updated_at: None,
    }))
}

pub async fn list_cash(pool: &SqlitePool, owner_id: &str) -> Result<Vec<CashBalance>> {
    let balances = sqlx::query_as::<_, CashBalance>(&format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? ORDER BY currency ASC"
    ))
    .bind(owner_id)
    .fetch_all(pool)
    .await?;

    Ok(balances)
}

pub async fn credit_cash(
    pool: &SqlitePool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
) -> Result<CashBalance> {
    sqlx::query(
        "INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at) \
         VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(owner_id, currency) DO UPDATE SET \
            balance_minor = balance_minor + excluded.balance_minor, \
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(owner_id)
    .bind(currency)
    .bind(currency_exponent)
    .bind(amount_minor)
    .execute(pool)
    .await?;

    get_cash_or_zero(pool, owner_id, currency, currency_exponent).await
}

pub async fn deposit_cash(
    pool: &SqlitePool,
    owner_id: &str,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
) -> Result<CashBalance> {
    let cash = credit_cash(pool, owner_id, currency, currency_exponent, amount_minor).await?;
    let _ = crate::modules::portfolio::audit::record_audit_entry(
        pool, owner_id, "deposit", None, None, None, amount_minor, currency, currency_exponent, None, None,
    )
    .await;
    Ok(cash)
}

pub async fn debit_cash(
    pool: &SqlitePool,
    owner_id: &str,
    currency: &str,
    amount_minor: i64,
) -> Result<WithdrawOutcome> {
    let existing: Option<(i64, i64)> = sqlx::query_as(
        "SELECT id, balance_minor FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?",
    )
    .bind(owner_id)
    .bind(currency)
    .fetch_optional(pool)
    .await?;

    let (id, balance) = match existing {
        Some(row) => row,
        None => return Ok(WithdrawOutcome::InsufficientFunds { available_minor: 0 }),
    };

    if amount_minor > balance {
        return Ok(WithdrawOutcome::InsufficientFunds { available_minor: balance });
    }

    sqlx::query(
        "UPDATE portfolio_cash_balances SET balance_minor = balance_minor - ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(amount_minor)
    .bind(id)
    .execute(pool)
    .await?;

    let updated = sqlx::query_as::<_, CashBalance>(&format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE id = ?"
    ))
    .bind(id)
    .fetch_one(pool)
    .await?;

    Ok(WithdrawOutcome::Ok(updated))
}

pub async fn withdraw_cash(
    pool: &SqlitePool,
    owner_id: &str,
    currency: &str,
    amount_minor: i64,
) -> Result<WithdrawOutcome> {
    let outcome = debit_cash(pool, owner_id, currency, amount_minor).await?;
    if let WithdrawOutcome::Ok(ref cash) = outcome {
        let _ = crate::modules::portfolio::audit::record_audit_entry(
            pool, owner_id, "withdraw", None, None, None, amount_minor, currency, cash.currency_exponent, None, None,
        )
        .await;
    }
    Ok(outcome)
}
