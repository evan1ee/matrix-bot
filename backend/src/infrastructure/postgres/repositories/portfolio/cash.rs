use chrono::{DateTime, Utc};
use sqlx::PgConnection;

use super::audit::record_audit_entry_tx;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::models::{CashBalance, WithdrawOutcome};

pub async fn get_cash_or_zero(
    ex: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
    currency: &str,
    currency_exponent: i64,
) -> Result<CashBalance, sqlx::Error> {
    let found = sqlx::query!(
        r#"
        SELECT id, owner_id, currency, currency_exponent, balance_minor, updated_at
        FROM portfolio_cash_balances
        WHERE owner_id = $1 AND currency = $2
        "#,
        owner_id.as_uuid(),
        currency
    )
    .fetch_optional(ex)
    .await?;

    Ok(found
        .map(|r| CashBalance {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id),
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            balance_minor: r.balance_minor,
            updated_at: r.updated_at,
        })
        .unwrap_or(CashBalance {
            id: 0,
            owner_id: *owner_id,
            currency: currency.to_string(),
            currency_exponent,
            balance_minor: 0,
            updated_at: None,
        }))
}

pub async fn list_cash(
    ex: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
) -> Result<Vec<CashBalance>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, currency, currency_exponent, balance_minor, updated_at
        FROM portfolio_cash_balances
        WHERE owner_id = $1
        ORDER BY currency ASC
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(ex)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| CashBalance {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id),
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            balance_minor: r.balance_minor,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn credit_cash(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
) -> Result<CashBalance, sqlx::Error> {
    sqlx::query!(
        r#"
        INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at)
        VALUES ($1, $2, $3, $4, CURRENT_TIMESTAMP)
        ON CONFLICT(owner_id, currency) DO UPDATE SET
            balance_minor = portfolio_cash_balances.balance_minor + excluded.balance_minor,
            updated_at = CURRENT_TIMESTAMP
        "#,
        owner_id.as_uuid(),
        currency,
        currency_exponent,
        amount_minor
    )
    .execute(&mut *conn)
    .await?;

    get_cash_or_zero(&mut *conn, owner_id, currency, currency_exponent).await
}

pub async fn deposit_cash_at(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    currency: &str,
    currency_exponent: i64,
    amount_minor: i64,
    executed_at: Option<DateTime<Utc>>,
) -> Result<CashBalance, sqlx::Error> {
    let cash = credit_cash(conn, owner_id, currency, currency_exponent, amount_minor).await?;
    let _ = record_audit_entry_tx(
        conn,
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
    .await?;
    Ok(cash)
}

pub async fn debit_cash(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    currency: &str,
    amount_minor: i64,
) -> Result<WithdrawOutcome, sqlx::Error> {
    let existing = sqlx::query!(
        "SELECT id, balance_minor, currency_exponent FROM portfolio_cash_balances WHERE owner_id = $1 AND currency = $2 FOR UPDATE",
        owner_id.as_uuid(),
        currency
    )
    .fetch_optional(&mut *conn)
    .await?;

    let row = match existing {
        Some(r) => r,
        None => return Ok(WithdrawOutcome::InsufficientFunds { available_minor: 0 }),
    };

    if amount_minor > row.balance_minor {
        return Ok(WithdrawOutcome::InsufficientFunds {
            available_minor: row.balance_minor,
        });
    }

    sqlx::query!(
        "UPDATE portfolio_cash_balances SET balance_minor = balance_minor - $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2",
        amount_minor,
        row.id
    )
    .execute(&mut *conn)
    .await?;

    let updated = sqlx::query!(
        r#"
        SELECT id, owner_id, currency, currency_exponent, balance_minor, updated_at
        FROM portfolio_cash_balances
        WHERE id = $1
        "#,
        row.id
    )
    .fetch_one(&mut *conn)
    .await?;

    Ok(WithdrawOutcome::Ok(CashBalance {
        id: updated.id,
        owner_id: AccountId::from_uuid(updated.owner_id),
        currency: updated.currency,
        currency_exponent: updated.currency_exponent,
        balance_minor: updated.balance_minor,
        updated_at: updated.updated_at,
    }))
}

pub async fn withdraw_cash_at(
    conn: &mut PgConnection,
    owner_id: &AccountId,
    currency: &str,
    amount_minor: i64,
    executed_at: Option<DateTime<Utc>>,
) -> Result<WithdrawOutcome, sqlx::Error> {
    let outcome = debit_cash(conn, owner_id, currency, amount_minor).await?;
    if let WithdrawOutcome::Ok(ref cash) = outcome {
        let _ = record_audit_entry_tx(
            conn,
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
        .await?;
    }
    Ok(outcome)
}
