use chrono::Utc;
use sqlx::PgPool;

use super::cash::list_cash;
use super::holdings::list;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::history::DailySnapshot;
use crate::domain::portfolio::models::scale_product;

pub async fn record_snapshot(
    pool: &PgPool,
    owner_id: &AccountId,
    snapshot_date: &str,
    currency: &str,
    currency_exponent: i64,
    holdings_value_minor: i64,
    cash_minor: i64,
) -> Result<(), sqlx::Error> {
    let total_net_worth_minor = holdings_value_minor + cash_minor;

    sqlx::query!(
        r#"
        INSERT INTO portfolio_daily_snapshots (
            owner_id, snapshot_date, currency, currency_exponent,
            holdings_value_minor, cash_minor, total_net_worth_minor
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)
        ON CONFLICT(owner_id, snapshot_date, currency) DO UPDATE SET
            holdings_value_minor = excluded.holdings_value_minor,
            cash_minor = excluded.cash_minor,
            total_net_worth_minor = excluded.total_net_worth_minor,
            currency_exponent = excluded.currency_exponent
        "#,
        owner_id.as_uuid(),
        snapshot_date,
        currency,
        currency_exponent,
        holdings_value_minor,
        cash_minor,
        total_net_worth_minor
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn get_snapshots(
    pool: &PgPool,
    owner_id: &AccountId,
    currency: &str,
    cutoff_date: &str,
) -> Result<Vec<DailySnapshot>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, snapshot_date, currency, currency_exponent,
               holdings_value_minor, cash_minor, total_net_worth_minor, created_at
        FROM portfolio_daily_snapshots
        WHERE owner_id = $1 AND currency = $2 AND snapshot_date >= $3
        ORDER BY snapshot_date ASC
        "#,
        owner_id.as_uuid(),
        currency,
        cutoff_date
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| DailySnapshot {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id),
            snapshot_date: r.snapshot_date,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            holdings_value_minor: r.holdings_value_minor,
            cash_minor: r.cash_minor,
            total_net_worth_minor: r.total_net_worth_minor,
            created_at: r.created_at.unwrap_or_else(Utc::now),
        })
        .collect())
}

pub async fn capture_current_snapshot_for_owner(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<(), sqlx::Error> {
    let holdings = list(pool, owner_id).await?;
    let cash_list = list_cash(pool, owner_id).await?;
    let today = Utc::now().format("%Y-%m-%d").to_string();

    let mut currencies = std::collections::HashSet::new();
    for h in &holdings {
        currencies.insert(h.currency.clone());
    }
    for c in &cash_list {
        currencies.insert(c.currency.clone());
    }
    if currencies.is_empty() {
        currencies.insert("USD".to_string());
    }

    for cur in currencies {
        let cur_holdings: Vec<&_> = holdings.iter().filter(|h| h.currency == cur).collect();
        let cur_cash = cash_list.iter().find(|c| c.currency == cur);

        let mut holdings_value = 0i64;
        let mut exponent = 2i64;

        for h in cur_holdings {
            exponent = h.currency_exponent;
            if let Some(price) = h.current_price_minor {
                holdings_value += scale_product(h.quantity_minor, price);
            }
        }

        let cash_minor = cur_cash
            .map(|c| {
                exponent = c.currency_exponent;
                c.balance_minor
            })
            .unwrap_or(0);

        record_snapshot(
            pool,
            owner_id,
            &today,
            &cur,
            exponent,
            holdings_value,
            cash_minor,
        )
        .await?;
    }

    Ok(())
}
