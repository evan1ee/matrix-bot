use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use super::audit::record_audit_entry_tx;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::models::{
    cost_of, realized_pl_minor, weighted_avg_cost_minor, BuyOutcome, CashBalance, Holding,
    SellOutcome,
};

#[allow(clippy::too_many_arguments)]
pub async fn buy_at(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    symbol: &str,
    quantity_minor: i64,
    price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    note: Option<&str>,
    executed_at: Option<DateTime<Utc>>,
) -> Result<BuyOutcome, sqlx::Error> {
    let existing = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
               currency_exponent, current_price_minor, price_updated_at, note,
               created_at, updated_at
        FROM portfolio_holdings
        WHERE owner_id = $1 AND symbol = $2
        FOR UPDATE
        "#,
        owner_id.as_uuid(),
        symbol
    )
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(ref h) = existing {
        if h.currency != currency {
            return Ok(BuyOutcome::CurrencyMismatch {
                existing_currency: h.currency.clone(),
            });
        }
    }

    let needed = cost_of(quantity_minor, price_minor);
    let cash_row = sqlx::query!(
        r#"
        SELECT id, balance_minor
        FROM portfolio_cash_balances
        WHERE owner_id = $1 AND currency = $2
        FOR UPDATE
        "#,
        owner_id.as_uuid(),
        currency
    )
    .fetch_optional(&mut *conn)
    .await?;

    let (cash_id, balance) = match cash_row {
        Some(row) => (row.id, row.balance_minor),
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

    sqlx::query!(
        r#"
        UPDATE portfolio_cash_balances
        SET balance_minor = balance_minor - $1, updated_at = CURRENT_TIMESTAMP
        WHERE id = $2
        "#,
        needed,
        cash_id
    )
    .execute(&mut *conn)
    .await?;

    let cash_record = sqlx::query!(
        r#"
        SELECT id, owner_id, currency, currency_exponent, balance_minor, updated_at
        FROM portfolio_cash_balances
        WHERE id = $1
        "#,
        cash_id
    )
    .fetch_one(&mut *conn)
    .await?;

    let cash = CashBalance {
        id: cash_record.id,
        owner_id: AccountId::from_uuid(cash_record.owner_id),
        currency: cash_record.currency,
        currency_exponent: cash_record.currency_exponent,
        balance_minor: cash_record.balance_minor,
        updated_at: cash_record.updated_at,
    };

    let holding = match existing {
        None => {
            let created_at = executed_at.unwrap_or_else(Utc::now);
            let row = sqlx::query!(
                r#"
                INSERT INTO portfolio_holdings
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, current_price_minor, price_updated_at, note, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, $7, $8)
                RETURNING id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
                          currency_exponent, current_price_minor, price_updated_at, note,
                          created_at, updated_at
                "#,
                owner_id.as_uuid(),
                symbol,
                quantity_minor,
                price_minor,
                currency,
                currency_exponent,
                note,
                created_at
            )
            .fetch_one(&mut *conn)
            .await?;

            Holding {
                id: row.id,
                owner_id: AccountId::from_uuid(row.owner_id),
                symbol: row.symbol,
                quantity_minor: row.quantity_minor,
                avg_cost_minor: row.avg_cost_minor,
                currency: row.currency,
                currency_exponent: row.currency_exponent,
                current_price_minor: row.current_price_minor,
                price_updated_at: row.price_updated_at,
                note: row.note,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }
        Some(existing) => {
            let new_quantity = existing.quantity_minor + quantity_minor;
            let new_avg_cost = weighted_avg_cost_minor(
                existing.quantity_minor,
                existing.avg_cost_minor,
                quantity_minor,
                price_minor,
            );

            let row = sqlx::query!(
                r#"
                UPDATE portfolio_holdings
                SET quantity_minor = $1, avg_cost_minor = $2, note = COALESCE($3, note), updated_at = CURRENT_TIMESTAMP
                WHERE id = $4
                RETURNING id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
                          currency_exponent, current_price_minor, price_updated_at, note,
                          created_at, updated_at
                "#,
                new_quantity,
                new_avg_cost,
                note,
                existing.id
            )
            .fetch_one(&mut *conn)
            .await?;

            Holding {
                id: row.id,
                owner_id: AccountId::from_uuid(row.owner_id),
                symbol: row.symbol,
                quantity_minor: row.quantity_minor,
                avg_cost_minor: row.avg_cost_minor,
                currency: row.currency,
                currency_exponent: row.currency_exponent,
                current_price_minor: row.current_price_minor,
                price_updated_at: row.price_updated_at,
                note: row.note,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }
    };

    record_audit_entry_tx(
        conn,
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
    .await?;

    Ok(BuyOutcome::Ok {
        holding: Box::new(holding),
        cash,
    })
}

pub async fn sell_at(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    symbol: &str,
    quantity_minor: i64,
    sell_price_minor: i64,
    executed_at: Option<DateTime<Utc>>,
) -> Result<SellOutcome, sqlx::Error> {
    let existing = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
               currency_exponent, current_price_minor, price_updated_at, note,
               created_at, updated_at
        FROM portfolio_holdings
        WHERE owner_id = $1 AND symbol = $2
        FOR UPDATE
        "#,
        owner_id.as_uuid(),
        symbol
    )
    .fetch_optional(&mut *conn)
    .await?;

    let existing = match existing {
        Some(h) => h,
        None => return Ok(SellOutcome::NotFound),
    };

    if quantity_minor > existing.quantity_minor {
        return Ok(SellOutcome::ExceedsHolding {
            held_minor: existing.quantity_minor,
        });
    }

    let realized = realized_pl_minor(quantity_minor, sell_price_minor, existing.avg_cost_minor);
    let proceeds = cost_of(quantity_minor, sell_price_minor);

    sqlx::query!(
        r#"
        INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at)
        VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
        ON CONFLICT(owner_id, currency) DO UPDATE SET
            balance_minor = portfolio_cash_balances.balance_minor + excluded.balance_minor,
            updated_at = CURRENT_TIMESTAMP
        "#,
        owner_id.as_uuid(),
        existing.currency,
        existing.currency_exponent,
        proceeds
    )
    .execute(&mut *conn)
    .await?;

    let cash_record = sqlx::query!(
        r#"
        SELECT id, owner_id, currency, currency_exponent, balance_minor, updated_at
        FROM portfolio_cash_balances
        WHERE owner_id = $1 AND currency = $2
        "#,
        owner_id.as_uuid(),
        existing.currency
    )
    .fetch_one(&mut *conn)
    .await?;

    let cash = CashBalance {
        id: cash_record.id,
        owner_id: AccountId::from_uuid(cash_record.owner_id),
        currency: cash_record.currency,
        currency_exponent: cash_record.currency_exponent,
        balance_minor: cash_record.balance_minor,
        updated_at: cash_record.updated_at,
    };

    let remaining = existing.quantity_minor - quantity_minor;
    let outcome = if remaining == 0 {
        sqlx::query!("DELETE FROM portfolio_holdings WHERE id = $1", existing.id)
            .execute(&mut *conn)
            .await?;
        SellOutcome::Closed {
            realized_pl_minor: realized,
            cash,
        }
    } else {
        let row = sqlx::query!(
            r#"
            UPDATE portfolio_holdings
            SET quantity_minor = $1, updated_at = CURRENT_TIMESTAMP
            WHERE id = $2
            RETURNING id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
                      currency_exponent, current_price_minor, price_updated_at, note,
                      created_at, updated_at
            "#,
            remaining,
            existing.id
        )
        .fetch_one(&mut *conn)
        .await?;

        let holding = Holding {
            id: row.id,
            owner_id: AccountId::from_uuid(row.owner_id),
            symbol: row.symbol,
            quantity_minor: row.quantity_minor,
            avg_cost_minor: row.avg_cost_minor,
            currency: row.currency,
            currency_exponent: row.currency_exponent,
            current_price_minor: row.current_price_minor,
            price_updated_at: row.price_updated_at,
            note: row.note,
            created_at: row.created_at,
            updated_at: row.updated_at,
        };

        SellOutcome::Reduced {
            holding,
            realized_pl_minor: realized,
            cash,
        }
    };

    record_audit_entry_tx(
        conn,
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
    .await?;

    Ok(outcome)
}

#[allow(clippy::too_many_arguments)]
pub async fn record_holding_action(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    symbol: &str,
    quantity_minor: i64,
    price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    note: Option<&str>,
    action: &str,
    executed_at: Option<DateTime<Utc>>,
) -> Result<Result<Holding, String>, sqlx::Error> {
    let existing = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
               currency_exponent, current_price_minor, price_updated_at, note,
               created_at, updated_at
        FROM portfolio_holdings
        WHERE owner_id = $1 AND symbol = $2
        FOR UPDATE
        "#,
        owner_id.as_uuid(),
        symbol
    )
    .fetch_optional(&mut *conn)
    .await?;

    if let Some(ref h) = existing {
        if h.currency != currency {
            return Ok(Err(h.currency.clone()));
        }
    }

    let total_val = cost_of(quantity_minor, price_minor);

    let holding = match existing {
        None => {
            let created_at = executed_at.unwrap_or_else(Utc::now);
            let row = sqlx::query!(
                r#"
                INSERT INTO portfolio_holdings
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, current_price_minor, price_updated_at, note, created_at)
                VALUES ($1, $2, $3, $4, $5, $6, NULL, NULL, $7, $8)
                RETURNING id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
                          currency_exponent, current_price_minor, price_updated_at, note,
                          created_at, updated_at
                "#,
                owner_id.as_uuid(),
                symbol,
                quantity_minor,
                price_minor,
                currency,
                currency_exponent,
                note,
                created_at
            )
            .fetch_one(&mut *conn)
            .await?;

            Holding {
                id: row.id,
                owner_id: AccountId::from_uuid(row.owner_id),
                symbol: row.symbol,
                quantity_minor: row.quantity_minor,
                avg_cost_minor: row.avg_cost_minor,
                currency: row.currency,
                currency_exponent: row.currency_exponent,
                current_price_minor: row.current_price_minor,
                price_updated_at: row.price_updated_at,
                note: row.note,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }
        Some(existing) => {
            let new_quantity = existing.quantity_minor + quantity_minor;
            let new_avg_cost = weighted_avg_cost_minor(
                existing.quantity_minor,
                existing.avg_cost_minor,
                quantity_minor,
                price_minor,
            );

            let row = sqlx::query!(
                r#"
                UPDATE portfolio_holdings
                SET quantity_minor = $1, avg_cost_minor = $2, note = COALESCE($3, note), updated_at = CURRENT_TIMESTAMP
                WHERE id = $4
                RETURNING id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
                          currency_exponent, current_price_minor, price_updated_at, note,
                          created_at, updated_at
                "#,
                new_quantity,
                new_avg_cost,
                note,
                existing.id
            )
            .fetch_one(&mut *conn)
            .await?;

            Holding {
                id: row.id,
                owner_id: AccountId::from_uuid(row.owner_id),
                symbol: row.symbol,
                quantity_minor: row.quantity_minor,
                avg_cost_minor: row.avg_cost_minor,
                currency: row.currency,
                currency_exponent: row.currency_exponent,
                current_price_minor: row.current_price_minor,
                price_updated_at: row.price_updated_at,
                note: row.note,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }
    };

    record_audit_entry_tx(
        conn,
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
    .await?;

    Ok(Ok(holding))
}
