//! Portfolio trading operations (buy, sell, record) with atomic database transactions.

use crate::db::DbPool;
use anyhow::Result;

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
    pool: &DbPool,
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

    let existing = crate::tx_fetch_optional!(
        &mut tx,
        Holding,
        &format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
        ),
        owner_id,
        symbol
    )?;

    if let Some(ref h) = existing {
        if h.currency != currency {
            return Ok(BuyOutcome::CurrencyMismatch {
                existing_currency: h.currency.clone(),
            });
        }
    }

    let needed = cost_of(quantity_minor, price_minor);
    let cash_row: Option<(i64, i64)> = crate::tx_fetch_optional!(
        &mut tx,
        _,
        "SELECT id, balance_minor FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?",
        owner_id,
        currency
    )?;

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

    crate::tx_execute!(&mut tx, "UPDATE portfolio_cash_balances SET balance_minor = balance_minor - ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", needed, cash_id)?;

    let cash = crate::tx_fetch_one!(
        &mut tx,
        CashBalance,
        &format!("SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE id = ?"),
        cash_id
    )?;

    let holding = match existing {
        None => {
            let created_at = executed_at.unwrap_or_else(chrono::Utc::now);
            crate::tx_execute!(&mut tx, "INSERT INTO portfolio_holdings \
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, current_price_minor, price_updated_at, note, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?)", owner_id, symbol, quantity_minor, price_minor, currency, currency_exponent, note, created_at)?;

            crate::tx_fetch_one!(
                &mut tx,
                Holding,
                &format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
            ),
                owner_id,
                symbol
            )?
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
                crate::tx_execute!(&mut tx, "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, note = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", new_quantity, new_avg_cost, text, existing.id)?;
            } else {
                crate::tx_execute!(&mut tx, "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", new_quantity, new_avg_cost, existing.id)?;
            }

            crate::tx_fetch_one!(
                &mut tx,
                Holding,
                &format!("SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ?"),
                existing.id
            )?
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

/// A buy executed now. See [`buy_at`] to backdate one.
#[allow(clippy::too_many_arguments)]
pub async fn buy(
    pool: &DbPool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    note: Option<&str>,
) -> Result<BuyOutcome> {
    buy_at(
        pool,
        owner_id,
        symbol,
        quantity_minor,
        price_minor,
        currency,
        currency_exponent,
        note,
        None,
    )
    .await
}

pub async fn sell_at(
    pool: &DbPool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    sell_price_minor: i64,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Result<SellOutcome> {
    let mut tx = pool.begin().await?;

    let existing = crate::tx_fetch_optional!(
        &mut tx,
        Holding,
        &format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
        ),
        owner_id,
        symbol
    )?;

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

    crate::tx_execute!(&mut tx, "INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at) \
         VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP) \
         ON CONFLICT(owner_id, currency) DO UPDATE SET \
            balance_minor = portfolio_cash_balances.balance_minor + excluded.balance_minor, \
            updated_at = CURRENT_TIMESTAMP", owner_id, &existing.currency, existing.currency_exponent, proceeds)?;

    let cash = crate::tx_fetch_one!(
        &mut tx,
        CashBalance,
        &format!(
        "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?"
    ),
        owner_id,
        &existing.currency
    )?;

    let remaining = existing.quantity_minor - quantity_minor;
    let outcome = if remaining == 0 {
        crate::tx_execute!(
            &mut tx,
            "DELETE FROM portfolio_holdings WHERE id = ?",
            existing.id
        )?;
        SellOutcome::Closed {
            realized_pl_minor: realized,
            cash,
        }
    } else {
        crate::tx_execute!(&mut tx, "UPDATE portfolio_holdings SET quantity_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", remaining, existing.id)?;

        let holding = crate::tx_fetch_one!(
            &mut tx,
            Holding,
            &format!("SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ?"),
            existing.id
        )?;

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

/// A sale executed now. See [`sell_at`] to backdate one.
pub async fn sell(
    pool: &DbPool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    sell_price_minor: i64,
) -> Result<SellOutcome> {
    sell_at(
        pool,
        owner_id,
        symbol,
        quantity_minor,
        sell_price_minor,
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn record_holding_action(
    pool: &DbPool,
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

    let existing = crate::tx_fetch_optional!(
        &mut tx,
        Holding,
        &format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
        ),
        owner_id,
        symbol
    )?;

    if let Some(ref h) = existing {
        if h.currency != currency {
            return Ok(Err(h.currency.clone()));
        }
    }

    let total_val = cost_of(quantity_minor, price_minor);

    let holding = match existing {
        None => {
            let created_at = executed_at.unwrap_or_else(chrono::Utc::now);
            crate::tx_execute!(&mut tx, "INSERT INTO portfolio_holdings \
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, current_price_minor, price_updated_at, note, created_at) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?, ?)", owner_id, symbol, quantity_minor, price_minor, currency, currency_exponent, note, created_at)?;

            crate::tx_fetch_one!(
                &mut tx,
                Holding,
                &format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
            ),
                owner_id,
                symbol
            )?
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
                crate::tx_execute!(&mut tx, "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, note = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", new_quantity, new_avg_cost, text, existing.id)?;
            } else {
                crate::tx_execute!(&mut tx, "UPDATE portfolio_holdings SET quantity_minor = ?, avg_cost_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?", new_quantity, new_avg_cost, existing.id)?;
            }

            crate::tx_fetch_one!(
                &mut tx,
                Holding,
                &format!("SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ?"),
                existing.id
            )?
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

/// Records an existing position without checking or debiting cash.
///
/// The audit action is `"record"` rather than `"buy"`: no money moved, so
/// calling it a purchase would put a trade in the ledger that never happened.
#[allow(clippy::too_many_arguments)]
pub async fn record_holding(
    pool: &DbPool,
    owner_id: &str,
    symbol: &str,
    quantity_minor: i64,
    price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    note: Option<&str>,
) -> Result<Result<Holding, String>> {
    record_holding_action(
        pool,
        owner_id,
        symbol,
        quantity_minor,
        price_minor,
        currency,
        currency_exponent,
        note,
        "record",
        None,
    )
    .await
}
