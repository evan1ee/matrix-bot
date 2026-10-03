//! Cross-feature read model for Net Worth calculations across finance and portfolio.
//!
//! Aggregates balances from transactions, portfolio holdings, and cash deposits.
//! Preserves stored minor units and dynamically converts to primary currency via FX rates.

use chrono::Utc;
use sqlx::PgPool;
use std::collections::HashMap;

use crate::domain::accounts::AccountId;
use crate::domain::finance::fx::convert_minor_units;
use crate::domain::finance::money::exponent_for;
use crate::domain::finance::net_worth::{CurrencyNetWorthBreakdown, NetWorthResponse};
use crate::infrastructure::web::fx::get_rates;

pub const QUANTITY_EXPONENT: u32 = 8;

pub async fn compute_net_worth(
    pool: &PgPool,
    http_client: &reqwest::Client,
    owner_id: &AccountId,
    primary_currency: &str,
) -> Result<NetWorthResponse, sqlx::Error> {
    let primary_curr = primary_currency.trim().to_ascii_uppercase();
    let primary_exponent = exponent_for(&primary_curr) as i64;
    let rates = get_rates(http_client).await;

    struct LedgerRow {
        currency: String,
        currency_exponent: i64,
        net_cash: Option<i64>,
    }

    let tx_rows = sqlx::query_as!(
        LedgerRow,
        r#"
        SELECT currency, currency_exponent,
               CAST(SUM(CASE WHEN direction = 'income' THEN amount_minor ELSE -amount_minor END) AS BIGINT) AS net_cash
        FROM transactions
        WHERE owner_id = $1
        GROUP BY currency, currency_exponent
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(pool)
    .await?;

    struct CashRow {
        currency: String,
        currency_exponent: i64,
        balance_minor: i64,
    }

    let cash_rows = sqlx::query_as!(
        CashRow,
        r#"
        SELECT currency, currency_exponent, balance_minor
        FROM portfolio_cash_balances
        WHERE owner_id = $1
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(pool)
    .await?;

    struct HoldingRow {
        currency: String,
        currency_exponent: i64,
        quantity_minor: i64,
        current_price_minor: Option<i64>,
    }

    let holding_rows = sqlx::query_as!(
        HoldingRow,
        r#"
        SELECT currency, currency_exponent, quantity_minor, current_price_minor
        FROM portfolio_holdings
        WHERE owner_id = $1
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(pool)
    .await?;

    struct Acc {
        exponent: i64,
        ledger: i64,
        deposits: i64,
        portfolio: i64,
    }

    let mut map: HashMap<String, Acc> = HashMap::new();

    for r in tx_rows {
        let entry = map.entry(r.currency).or_insert(Acc {
            exponent: r.currency_exponent,
            ledger: 0,
            deposits: 0,
            portfolio: 0,
        });
        entry.ledger += r.net_cash.unwrap_or(0);
    }

    for r in cash_rows {
        let entry = map.entry(r.currency).or_insert(Acc {
            exponent: r.currency_exponent,
            ledger: 0,
            deposits: 0,
            portfolio: 0,
        });
        entry.deposits += r.balance_minor;
    }

    for r in holding_rows {
        let price = r.current_price_minor.unwrap_or(0);
        let market_val = if price > 0 && r.quantity_minor > 0 {
            let divisor = 10i128.pow(QUANTITY_EXPONENT);
            ((r.quantity_minor as i128 * price as i128) / divisor) as i64
        } else {
            0
        };

        let entry = map.entry(r.currency).or_insert(Acc {
            exponent: r.currency_exponent,
            ledger: 0,
            deposits: 0,
            portfolio: 0,
        });
        entry.portfolio += market_val;
    }

    if map.is_empty() {
        map.insert(
            primary_curr.clone(),
            Acc {
                exponent: primary_exponent,
                ledger: 0,
                deposits: 0,
                portfolio: 0,
            },
        );
    }

    let mut currencies = Vec::new();
    let mut total_net_worth_minor: i64 = 0;
    let mut total_ledger_minor: i64 = 0;
    let mut total_portfolio_minor: i64 = 0;
    let mut total_deposits_minor: i64 = 0;
    let mut relevant_fx_rates = HashMap::new();

    for (curr, acc) in map {
        let total_minor = acc.ledger + acc.deposits + acc.portfolio;
        let rate_to_primary =
            crate::domain::finance::fx::get_rate_between(&curr, &primary_curr, &rates);
        relevant_fx_rates.insert(curr.clone(), rate_to_primary);

        let converted_minor = convert_minor_units(total_minor, &curr, &primary_curr, &rates);

        total_net_worth_minor += converted_minor;
        total_ledger_minor += convert_minor_units(acc.ledger, &curr, &primary_curr, &rates);
        total_portfolio_minor += convert_minor_units(acc.portfolio, &curr, &primary_curr, &rates);
        total_deposits_minor += convert_minor_units(acc.deposits, &curr, &primary_curr, &rates);

        currencies.push(CurrencyNetWorthBreakdown {
            currency: curr,
            currency_exponent: acc.exponent,
            ledger_cash_minor: acc.ledger,
            portfolio_market_value_minor: acc.portfolio,
            cash_deposits_minor: acc.deposits,
            total_minor,
            fx_rate_to_primary: rate_to_primary,
            converted_minor,
        });
    }

    currencies.sort_by_key(|c| std::cmp::Reverse(c.converted_minor));

    Ok(NetWorthResponse {
        primary_currency: primary_curr,
        primary_currency_exponent: primary_exponent,
        total_net_worth_minor,
        ledger_cash_minor: total_ledger_minor,
        portfolio_market_value_minor: total_portfolio_minor,
        cash_deposits_minor: total_deposits_minor,
        currencies,
        fx_rates: relevant_fx_rates,
        updated_at: Utc::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_compute_net_worth_aggregation() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let user_id = AccountId::from_seed("user1");

        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'User', 'active') ON CONFLICT DO NOTHING",
            user_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        // Insert a transaction (ledger cash: $50.00 USD)
        sqlx::query!(
            r#"
            INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, currency, currency_exponent, occurred_at)
            VALUES ('web', $1, 'User', 'income', 5000, 'USD', 2, CURRENT_TIMESTAMP)
            "#,
            user_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        // Insert cash balance ($100.00 USD)
        sqlx::query!(
            r#"
            INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor)
            VALUES ($1, 'USD', 2, 10000)
            "#,
            user_id.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let client = reqwest::Client::new();
        let res = compute_net_worth(&pool, &client, &user_id, "USD")
            .await
            .unwrap();

        assert_eq!(res.primary_currency, "USD");
        assert_eq!(res.ledger_cash_minor, 5000);
        assert_eq!(res.cash_deposits_minor, 10000);
        assert_eq!(res.total_net_worth_minor, 15000);
    }
}
