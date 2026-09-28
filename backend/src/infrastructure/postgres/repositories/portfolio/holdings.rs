use sqlx::PgPool;

use super::cash::list_cash;
use crate::domain::portfolio::models::{
    build_summaries, normalise_symbol, Holding, PortfolioCurrencySummary,
};

pub async fn find(
    pool: &PgPool,
    owner_id: &str,
    symbol: &str,
) -> Result<Option<Holding>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
               currency_exponent, current_price_minor, price_updated_at, note,
               created_at, updated_at
        FROM portfolio_holdings
        WHERE owner_id = $1 AND symbol = $2
        "#,
        owner_id,
        symbol
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Holding {
        id: r.id,
        owner_id: r.owner_id,
        symbol: r.symbol,
        quantity_minor: r.quantity_minor,
        avg_cost_minor: r.avg_cost_minor,
        currency: r.currency,
        currency_exponent: r.currency_exponent,
        current_price_minor: r.current_price_minor,
        price_updated_at: r.price_updated_at,
        note: r.note,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

pub async fn find_owned(
    pool: &PgPool,
    id: i64,
    owner_id: &str,
) -> Result<Option<Holding>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
               currency_exponent, current_price_minor, price_updated_at, note,
               created_at, updated_at
        FROM portfolio_holdings
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| Holding {
        id: r.id,
        owner_id: r.owner_id,
        symbol: r.symbol,
        quantity_minor: r.quantity_minor,
        avg_cost_minor: r.avg_cost_minor,
        currency: r.currency,
        currency_exponent: r.currency_exponent,
        current_price_minor: r.current_price_minor,
        price_updated_at: r.price_updated_at,
        note: r.note,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

pub async fn list(pool: &PgPool, owner_id: &str) -> Result<Vec<Holding>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, symbol, quantity_minor, avg_cost_minor, currency,
               currency_exponent, current_price_minor, price_updated_at, note,
               created_at, updated_at
        FROM portfolio_holdings
        WHERE owner_id = $1
        ORDER BY symbol ASC
        "#,
        owner_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| Holding {
            id: r.id,
            owner_id: r.owner_id,
            symbol: r.symbol,
            quantity_minor: r.quantity_minor,
            avg_cost_minor: r.avg_cost_minor,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            current_price_minor: r.current_price_minor,
            price_updated_at: r.price_updated_at,
            note: r.note,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn set_price(
    pool: &PgPool,
    owner_id: &str,
    symbol: &str,
    price_minor: i64,
) -> Result<Option<Holding>, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        UPDATE portfolio_holdings
        SET current_price_minor = $1, price_updated_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP
        WHERE owner_id = $2 AND symbol = $3
        "#,
        price_minor,
        owner_id,
        symbol
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }
    find(pool, owner_id, symbol).await
}

pub async fn set_note(
    pool: &PgPool,
    owner_id: &str,
    symbol: &str,
    note: Option<&str>,
) -> Result<Option<Holding>, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        UPDATE portfolio_holdings
        SET note = $1, updated_at = CURRENT_TIMESTAMP
        WHERE owner_id = $2 AND symbol = $3
        "#,
        note,
        owner_id,
        symbol
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }
    find(pool, owner_id, symbol).await
}

pub async fn update_symbol(
    pool: &PgPool,
    owner_id: &str,
    id: i64,
    new_symbol: &str,
) -> Result<Result<Holding, String>, sqlx::Error> {
    let normalized = match normalise_symbol(new_symbol) {
        Ok(sym) => sym,
        Err(e) => return Ok(Err(e)),
    };

    if let Some(existing) = find(pool, owner_id, &normalized).await? {
        if existing.id != id {
            return Ok(Err(format!("Position in {} already exists", normalized)));
        }
    }

    let result = sqlx::query!(
        r#"
        UPDATE portfolio_holdings
        SET symbol = $1, updated_at = CURRENT_TIMESTAMP
        WHERE id = $2 AND owner_id = $3
        "#,
        normalized,
        id,
        owner_id
    )
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(Err("Holding not found or access denied".to_string()));
    }

    match find_owned(pool, id, owner_id).await? {
        Some(h) => Ok(Ok(h)),
        None => Ok(Err("Updated holding vanished".to_string())),
    }
}

pub async fn delete(pool: &PgPool, owner_id: &str, symbol: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        DELETE FROM portfolio_holdings
        WHERE owner_id = $1 AND symbol = $2
        "#,
        owner_id,
        symbol
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn summary(
    pool: &PgPool,
    owner_id: &str,
) -> Result<Vec<PortfolioCurrencySummary>, sqlx::Error> {
    let holdings = list(pool, owner_id).await?;
    let cash_balances = list_cash(pool, owner_id).await?;
    Ok(build_summaries(&holdings, &cash_balances))
}

pub async fn list_holding_owners(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_scalar!(
        r#"
        SELECT DISTINCT owner_id
        FROM portfolio_holdings
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
