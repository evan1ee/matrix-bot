//! Net worth calculation across finance ledger, portfolio holdings, and cash deposits.
//!
//! Aggregates balances from all three financial pillars, preserves all stored minor
//! units without modification, and converts totals dynamically into the user's primary
//! target currency using daily ECB / Yahoo FX reference rates.

use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::modules::finance::fx::{convert_minor_units, get_rate_between, get_rates};
use crate::modules::finance::money::exponent_for;
use crate::modules::portfolio::models::QUANTITY_EXPONENT;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CurrencyNetWorthBreakdown {
    pub currency: String,
    pub currency_exponent: i64,
    pub ledger_cash_minor: i64,
    pub portfolio_market_value_minor: i64,
    pub cash_deposits_minor: i64,
    pub total_minor: i64,
    pub fx_rate_to_primary: f64,
    pub converted_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NetWorthResponse {
    pub primary_currency: String,
    pub primary_currency_exponent: i64,
    pub total_net_worth_minor: i64,
    pub ledger_cash_minor: i64,
    pub portfolio_market_value_minor: i64,
    pub cash_deposits_minor: i64,
    pub currencies: Vec<CurrencyNetWorthBreakdown>,
    pub fx_rates: HashMap<String, f64>,
    pub updated_at: DateTime<Utc>,
}

pub struct NetWorthService;

impl NetWorthService {
    pub async fn compute_net_worth(
        pool: &DbPool,
        http_client: &reqwest::Client,
        owner_id: &str,
        primary_currency: &str,
    ) -> Result<NetWorthResponse> {
        let primary_curr = primary_currency.trim().to_ascii_uppercase();
        let primary_exponent = exponent_for(&primary_curr) as i64;
        let rates = get_rates(http_client).await;

        // 1. Ledger Cash from transactions (all-time net: income - expense)
        let tx_rows = crate::db_fetch_all!(
            pool,
            (String, i64, Option<i64>),
            r#"
            SELECT currency, currency_exponent,
                   CAST(SUM(CASE WHEN direction = 'income' THEN amount_minor ELSE -amount_minor END) AS BIGINT)
            FROM transactions
            WHERE owner_id = ?
            GROUP BY currency, currency_exponent
            "#,
            owner_id
        )?;

        // 2. Cash Deposits from portfolio_cash_balances
        let cash_rows = crate::db_fetch_all!(
            pool,
            (String, i64, i64),
            r#"
            SELECT currency, currency_exponent, balance_minor
            FROM portfolio_cash_balances
            WHERE owner_id = ?
            "#,
            owner_id
        )
        .unwrap_or_default();

        // 3. Portfolio Holdings market value
        let holding_rows = crate::db_fetch_all!(
            pool,
            (String, i64, i64, Option<i64>),
            r#"
            SELECT currency, currency_exponent, quantity_minor, current_price_minor
            FROM holdings
            WHERE owner_id = ?
            "#,
            owner_id
        )
        .unwrap_or_default();

        struct PillarAccumulator {
            exponent: i64,
            ledger_cash: i64,
            portfolio_market_val: i64,
            cash_deposits: i64,
        }

        let mut per_currency: HashMap<String, PillarAccumulator> = HashMap::new();

        for (curr, exp, net_opt) in tx_rows {
            let entry =
                per_currency
                    .entry(curr.to_ascii_uppercase())
                    .or_insert(PillarAccumulator {
                        exponent: exp,
                        ledger_cash: 0,
                        portfolio_market_val: 0,
                        cash_deposits: 0,
                    });
            entry.ledger_cash += net_opt.unwrap_or(0);
        }

        for (curr, exp, bal) in cash_rows {
            let entry =
                per_currency
                    .entry(curr.to_ascii_uppercase())
                    .or_insert(PillarAccumulator {
                        exponent: exp,
                        ledger_cash: 0,
                        portfolio_market_val: 0,
                        cash_deposits: 0,
                    });
            entry.cash_deposits += bal;
        }

        for (curr, exp, qty_minor, price_minor_opt) in holding_rows {
            let price = price_minor_opt.unwrap_or(0);
            let market_val = if price > 0 && qty_minor > 0 {
                let divisor = 10i128.pow(QUANTITY_EXPONENT as u32);
                ((qty_minor as i128 * price as i128) / divisor) as i64
            } else {
                0
            };
            let entry =
                per_currency
                    .entry(curr.to_ascii_uppercase())
                    .or_insert(PillarAccumulator {
                        exponent: exp,
                        ledger_cash: 0,
                        portfolio_market_val: 0,
                        cash_deposits: 0,
                    });
            entry.portfolio_market_val += market_val;
        }

        // Ensure primary currency is represented if empty
        if per_currency.is_empty() {
            per_currency.insert(
                primary_curr.clone(),
                PillarAccumulator {
                    exponent: primary_exponent,
                    ledger_cash: 0,
                    portfolio_market_val: 0,
                    cash_deposits: 0,
                },
            );
        }

        let mut breakdowns = Vec::new();
        let mut total_net_worth_minor = 0i64;
        let mut total_ledger_cash_minor = 0i64;
        let mut total_portfolio_market_val_minor = 0i64;
        let mut total_cash_deposits_minor = 0i64;

        let mut relevant_fx_rates = HashMap::new();

        for (curr, accum) in per_currency {
            let rate = get_rate_between(&rates, &curr, &primary_curr).unwrap_or(1.0);
            relevant_fx_rates.insert(curr.clone(), rate);

            let total_native = accum.ledger_cash + accum.portfolio_market_val + accum.cash_deposits;

            let converted_ledger = convert_minor_units(
                accum.ledger_cash,
                accum.exponent as u32,
                primary_exponent as u32,
                rate,
            );
            let converted_portfolio = convert_minor_units(
                accum.portfolio_market_val,
                accum.exponent as u32,
                primary_exponent as u32,
                rate,
            );
            let converted_deposits = convert_minor_units(
                accum.cash_deposits,
                accum.exponent as u32,
                primary_exponent as u32,
                rate,
            );
            let converted_total = convert_minor_units(
                total_native,
                accum.exponent as u32,
                primary_exponent as u32,
                rate,
            );

            total_ledger_cash_minor += converted_ledger;
            total_portfolio_market_val_minor += converted_portfolio;
            total_cash_deposits_minor += converted_deposits;
            total_net_worth_minor += converted_total;

            breakdowns.push(CurrencyNetWorthBreakdown {
                currency: curr,
                currency_exponent: accum.exponent,
                ledger_cash_minor: accum.ledger_cash,
                portfolio_market_value_minor: accum.portfolio_market_val,
                cash_deposits_minor: accum.cash_deposits,
                total_minor: total_native,
                fx_rate_to_primary: rate,
                converted_minor: converted_total,
            });
        }

        breakdowns.sort_by(|a, b| b.converted_minor.cmp(&a.converted_minor));

        Ok(NetWorthResponse {
            primary_currency: primary_curr,
            primary_currency_exponent: primary_exponent,
            total_net_worth_minor,
            ledger_cash_minor: total_ledger_cash_minor,
            portfolio_market_value_minor: total_portfolio_market_val_minor,
            cash_deposits_minor: total_cash_deposits_minor,
            currencies: breakdowns,
            fx_rates: relevant_fx_rates,
            updated_at: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn test_compute_net_worth_aggregation() {
        let pool = crate::db::DbPool::Sqlite(
            SqlitePoolOptions::new()
                .connect("sqlite::memory:")
                .await
                .unwrap(),
        );

        crate::db::schema::run_migrations(&pool).await.unwrap();

        // Insert a transaction (ledger cash: $50.00 USD)
        crate::db_execute!(&pool, r#"
            INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, currency, currency_exponent, occurred_at)
            VALUES ('web', 'user1', 'User', 'income', 5000, 'USD', 2, CURRENT_TIMESTAMP)
            "#)
        .unwrap();

        // Insert cash balance ($100.00 USD)
        crate::db_execute!(&pool, r#"
            INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor)
            VALUES ('user1', 'USD', 2, 10000)
            "#)
        .unwrap();

        let client = reqwest::Client::new();
        let res = NetWorthService::compute_net_worth(&pool, &client, "user1", "USD")
            .await
            .unwrap();

        assert_eq!(res.primary_currency, "USD");
        assert_eq!(res.ledger_cash_minor, 5000);
        assert_eq!(res.cash_deposits_minor, 10000);
        assert_eq!(res.total_net_worth_minor, 15000);
    }
}
