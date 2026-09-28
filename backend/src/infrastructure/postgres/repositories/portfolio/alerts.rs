use chrono::Utc;
use sqlx::PgPool;

use crate::domain::portfolio::alerts::{AlertCondition, PriceAlert};

pub async fn create(
    pool: &PgPool,
    owner_id: &str,
    symbol: &str,
    target_price_minor: i64,
    currency: &str,
    currency_exponent: i64,
    condition: AlertCondition,
) -> Result<PriceAlert, sqlx::Error> {
    let norm_symbol = symbol.trim().to_uppercase();
    let norm_currency = currency.trim().to_uppercase();

    let row = sqlx::query!(
        r#"
        INSERT INTO portfolio_price_alerts (
            owner_id, symbol, target_price_minor, currency, currency_exponent, condition
        ) VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id, owner_id, symbol, target_price_minor, currency, currency_exponent,
                  condition, is_triggered, triggered_at, created_at
        "#,
        owner_id,
        norm_symbol,
        target_price_minor,
        norm_currency,
        currency_exponent,
        condition.as_str()
    )
    .fetch_one(pool)
    .await?;

    Ok(PriceAlert {
        id: row.id,
        owner_id: row.owner_id,
        symbol: row.symbol,
        target_price_minor: row.target_price_minor,
        currency: row.currency,
        currency_exponent: row.currency_exponent,
        condition: row.condition,
        is_triggered: row.is_triggered,
        triggered_at: row.triggered_at,
        created_at: row.created_at.unwrap_or_else(Utc::now),
    })
}

pub async fn find_by_id(
    pool: &PgPool,
    id: i64,
    owner_id: &str,
) -> Result<Option<PriceAlert>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, target_price_minor, currency, currency_exponent,
               condition, is_triggered, triggered_at, created_at
        FROM portfolio_price_alerts
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| PriceAlert {
        id: r.id,
        owner_id: r.owner_id,
        symbol: r.symbol,
        target_price_minor: r.target_price_minor,
        currency: r.currency,
        currency_exponent: r.currency_exponent,
        condition: r.condition,
        is_triggered: r.is_triggered,
        triggered_at: r.triggered_at,
        created_at: r.created_at.unwrap_or_else(Utc::now),
    }))
}

pub async fn list_for_owner(pool: &PgPool, owner_id: &str) -> Result<Vec<PriceAlert>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, target_price_minor, currency, currency_exponent,
               condition, is_triggered, triggered_at, created_at
        FROM portfolio_price_alerts
        WHERE owner_id = $1
        ORDER BY is_triggered ASC, created_at DESC
        "#,
        owner_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| PriceAlert {
            id: r.id,
            owner_id: r.owner_id,
            symbol: r.symbol,
            target_price_minor: r.target_price_minor,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            condition: r.condition,
            is_triggered: r.is_triggered,
            triggered_at: r.triggered_at,
            created_at: r.created_at.unwrap_or_else(Utc::now),
        })
        .collect())
}

pub async fn list_active_for_symbol(
    pool: &PgPool,
    symbol: &str,
) -> Result<Vec<PriceAlert>, sqlx::Error> {
    let norm = symbol.trim().to_uppercase();
    let like_pattern = format!("{}-%", norm);
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, target_price_minor, currency, currency_exponent,
               condition, is_triggered, triggered_at, created_at
        FROM portfolio_price_alerts
        WHERE is_triggered = FALSE AND (symbol = $1 OR symbol LIKE $2)
        "#,
        norm,
        like_pattern
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| PriceAlert {
            id: r.id,
            owner_id: r.owner_id,
            symbol: r.symbol,
            target_price_minor: r.target_price_minor,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            condition: r.condition,
            is_triggered: r.is_triggered,
            triggered_at: r.triggered_at,
            created_at: r.created_at.unwrap_or_else(Utc::now),
        })
        .collect())
}

pub async fn list_active_symbols(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query!(
        "SELECT DISTINCT symbol FROM portfolio_price_alerts WHERE is_triggered = FALSE"
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.symbol).collect())
}

pub async fn delete(pool: &PgPool, id: i64, owner_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "DELETE FROM portfolio_price_alerts WHERE id = $1 AND owner_id = $2",
        id,
        owner_id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn mark_triggered(pool: &PgPool, id: i64) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        "UPDATE portfolio_price_alerts SET is_triggered = TRUE, triggered_at = CURRENT_TIMESTAMP WHERE id = $1",
        id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}
