//! Portfolio trading operations (buy, sell, record) with atomic database transactions.

use anyhow::Result;
use sqlx::SqlitePool;

use crate::modules::portfolio::models::{
    cost_of, realized_pl_minor, weighted_avg_cost_minor, BuyOutcome, CashBalance, Holding,
    SellOutcome,
};

const HOLDING_COLUMNS: &str = "id, owner_id, symbol, quantity_minor, avg_cost_minor, currency, \
                                currency_exponent, current_price_minor, price_updated_at, note, \
                                created_at, updated_at";
const CASH_COLUMNS: &str = "id, owner_id, currency, currency_exponent, balance_minor, updated_at";

#[allow(clippy::too_many_arguments)]
pub async fn buy_at(
    pool: &SqlitePool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    note: Option<&str>,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<BuyOutcome> {
    let mut tx = pool.begin().await?;

    let existing = sqlx::query_as::<_, Holding>(&format!(
        "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
    ))
    .bind(owner_id)
    .bind(symbol)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref h) = existing {
        if h.currency != currency {
            return Ok(BuyOutcome::CurrencyMismatch {
                existing_currency: h.currency.clone(),
            });
        }
    }

    let needed = cost_of(quantity_minor, price_minor);
    let cash_row: Option<(i64, i64)> = sqlx::query_as(
        "SELECT id, balance_minor FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?",
    )
    .bind(owner_id)
    .bind(currency)
    .fetch_optional(&mut *tx)
    .await?;

    let (cash_id, balance) = match cash_row {
        Some(row) => row,
        None => {
            return Ok(BuyOutcome::InsufficientCash {
                available_minor: 0,
                needed_minor: needed,
            });
        }
    };

    if needed > balance {
        return Ok(BuyOutcome::InsufficientCash {
            available_minor: balance,
            needed_minor: needed,
        });
    }

    sqlx::query(
        "UPDATE portfolio_cash_balances SET balance_minor = balance_minor - ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
    )
    .bind(needed)
    .bind(cash_id)
    .execute(&mut *tx)
    .await?;

    let cash = sqlx::query_as::<_, CashBalance>(&format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE id = ?"
    ))
    .bind(cash_id)
    .fetch_one(&mut *tx)
    .await?;

    let holding = match existing {
        None => {
            let created_at = executed_at.unwrap_or_else(chrono::Utc::now);
            sqlx::query(
                "INSERT INTO portfolio_holdings \
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, current_price_minor, price_updated_at, note, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?)",
            )
            .bind(owner_id)
            .bind(symbol)
            .bind(quantity_minor)
            .bind(price_minor)
            .bind(currency)
            .bind(currency_exponent)
            .bind(note)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;

            sqlx::query_as::<_, Holding>(&format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
            ))
            .bind(owner_id)
            .bind(symbol)
            .fetch_one(&mut *tx)
            .await?
        }
        Some(existing) => {
            let new_quantity = existing.quantity_minor + quantity_minor;
            let new_avg_cost = weighted_avg_cost_minor(
                existing.quantity_minor,
                existing.avg_cost_minor,
                quantity_minor,
                price_minor,
            );

            if let Some(text) = note {
                sqlx::query(
                    "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, note = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(new_quantity)
                .bind(new_avg_cost)
                .bind(text)
                .bind(existing.id)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query(
                    "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(new_quantity)
                .bind(new_avg_cost)
                .bind(existing.id)
                .execute(&mut *tx)
                .await?;
            }

            sqlx::query_as::<_, Holding>(&format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ?"
            ))
            .bind(existing.id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    tx.commit().await?;

    let _ = crate::modules::portfolio::audit::record_audit_entry(
        pool,
        owner_id,
        "buy",
        Some(symbol),
        Some(quantity_minor),
        Some(price_minor),
        needed,
        currency,
        currency_exponent,
        None,
        note,
        executed_at,
    )
    .await;

    Ok(BuyOutcome::Ok { holding, cash })
}

pub async fn sell_at(
    pool: &SqlitePool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    sell_price_minor: i64,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<SellOutcome> {
    let mut tx = pool.begin().await?;

    let existing = sqlx::query_as::<_, Holding>(&format!(
        "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
    ))
    .bind(owner_id)
    .bind(symbol)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(existing) = existing else {
        return Ok(SellOutcome::NotFound);
    };

    if quantity_minor > existing.quantity_minor {
        return Ok(SellOutcome::ExceedsHolding {
            held_minor: existing.quantity_minor,
        });
    }

    let realized = realized_pl_minor(quantity_minor, sell_price_minor, existing.avg_cost_minor);
    let proceeds = cost_of(quantity_minor, sell_price_minor);

    sqlx::query(
        "INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at) \
         VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(owner_id, currency) DO UPDATE SET \
            balance_minor = balance_minor + excluded.balance_minor, \
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(owner_id)
    .bind(&existing.currency)
    .bind(existing.currency_exponent)
    .bind(proceeds)
    .execute(&mut *tx)
    .await?;

    let cash = sqlx::query_as::<_, CashBalance>(&format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?"
    ))
    .bind(owner_id)
    .bind(&existing.currency)
    .fetch_one(&mut *tx)
    .await?;

    let remaining = existing.quantity_minor - quantity_minor;
    let outcome = if remaining == 0 {
        sqlx::query("DELETE FROM portfolio_holdings WHERE id = ?")
            .bind(existing.id)
            .execute(&mut *tx)
            .await?;
        SellOutcome::Closed {
            realized_pl_minor: realized,
            cash,
        }
    } else {
        sqlx::query(
            "UPDATE portfolio_holdings SET quantity_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(remaining)
        .bind(existing.id)
        .execute(&mut *tx)
        .await?;

        let holding = sqlx::query_as::<_, Holding>(&format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ?"
        ))
        .bind(existing.id)
        .fetch_one(&mut *tx)
        .await?;

        SellOutcome::Reduced {
            holding,
            realized_pl_minor: realized,
            cash,
        }
    };

    tx.commit().await?;

    let _ = crate::modules::portfolio::audit::record_audit_entry(
        pool,
        owner_id,
        "sell",
        Some(symbol),
        Some(quantity_minor),
        Some(sell_price_minor),
        proceeds,
        &existing.currency,
        existing.currency_exponent,
        Some(realized),
        None,
        executed_at,
    )
    .await;

    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
pub async fn record_holding_action(
    pool: &SqlitePool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    note: Option<&str>,
    action: &str,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<Result<Holding, String>> {
    let mut tx = pool.begin().await?;

    let existing = sqlx::query_as::<_, Holding>(&format!(
        "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
    ))
    .bind(owner_id)
    .bind(symbol)
    .fetch_optional(&mut *tx)
    .await?;

    if let Some(ref h) = existing {
        if h.currency != currency {
            return Ok(Err(h.currency.clone()));
        }
    }

    let total_val = cost_of(quantity_minor, price_minor);

    let holding = match existing {
        None => {
            let created_at = executed_at.unwrap_or_else(chrono::Utc::now);
            sqlx::query(
                "INSERT INTO portfolio_holdings \
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, current_price_minor, price_updated_at, note, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?)",
            )
            .bind(owner_id)
            .bind(symbol)
            .bind(quantity_minor)
            .bind(price_minor)
            .bind(currency)
            .bind(currency_exponent)
            .bind(note)
            .bind(created_at)
            .execute(&mut *tx)
            .await?;

            sqlx::query_as::<_, Holding>(&format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
            ))
            .bind(owner_id)
            .bind(symbol)
            .fetch_one(&mut *tx)
            .await?
        }
        Some(existing) => {
            let new_quantity = existing.quantity_minor + quantity_minor;
            let new_avg_cost = weighted_avg_cost_minor(
                existing.quantity_minor,
                existing.avg_cost_minor,
                quantity_minor,
                price_minor,
            );

            if let Some(text) = note {
                sqlx::query(
                    "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, note = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(new_quantity)
                .bind(new_avg_cost)
                .bind(text)
                .bind(existing.id)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query(
                    "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(new_quantity)
                .bind(new_avg_cost)
                .bind(existing.id)
                .execute(&mut *tx)
                .await?;
            }

            sqlx::query_as::<_, Holding>(&format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ?"
            ))
            .bind(existing.id)
            .fetch_one(&mut *tx)
            .await?
        }
    };

    tx.commit().await?;

    let _ = crate::modules::portfolio::audit::record_audit_entry(
        pool,
        owner_id,
        action,
        Some(symbol),
        Some(quantity_minor),
        Some(price_minor),
        total_val,
        currency,
        currency_exponent,
        None,
        note,
        executed_at,
    )
    .await;

    Ok(Ok(holding))
}
