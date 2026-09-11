use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};
use utoipa::ToSchema;

use crate::modules::portfolio::models::Holding;
use crate::modules::portfolio::repository::PortfolioRepository;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct DailySnapshot {
    pub id: i64,
    pub owner_id: String,
    pub snapshot_date: String,
    pub currency: String,
    pub currency_exponent: i64,
    pub holdings_value_minor: i64,
    pub cash_minor: i64,
    pub total_net_worth_minor: i64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortfolioHistoryPoint {
    pub date: String,
    pub holdings_value_minor: i64,
    pub cash_minor: i64,
    pub total_net_worth_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortfolioHistoryResponse {
    pub currency: String,
    pub currency_exponent: i64,
    pub timeframe: String,
    pub points: Vec<PortfolioHistoryPoint>,
    pub starting_value_minor: i64,
    pub current_value_minor: i64,
    pub period_change_minor: i64,
    pub period_change_pct: f64,
}

pub struct HistoryRepository;

impl HistoryRepository {
    pub async fn record_snapshot(
        pool: &SqlitePool,
        owner_id: &str,
        snapshot_date: &str,
        currency: &str,
        currency_exponent: i64,
        holdings_value_minor: i64,
        cash_minor: i64,
    ) -> Result<()> {
        let total_net_worth_minor = holdings_value_minor + cash_minor;

        sqlx::query(
            r#"
            INSERT INTO portfolio_daily_snapshots (
                owner_id, snapshot_date, currency, currency_exponent,
                holdings_value_minor, cash_minor, total_net_worth_minor
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(owner_id, snapshot_date, currency) DO UPDATE SET
                holdings_value_minor = excluded.holdings_value_minor,
                cash_minor = excluded.cash_minor,
                total_net_worth_minor = excluded.total_net_worth_minor,
                currency_exponent = excluded.currency_exponent;
            "#,
        )
        .bind(owner_id)
        .bind(snapshot_date)
        .bind(currency)
        .bind(currency_exponent)
        .bind(holdings_value_minor)
        .bind(cash_minor)
        .bind(total_net_worth_minor)
        .execute(pool)
        .await?;

        Ok(())
    }

    pub async fn capture_current_snapshot_for_owner(
        pool: &SqlitePool,
        owner_id: &str,
    ) -> Result<()> {
        let holdings = PortfolioRepository::list(pool, owner_id).await?;
        let cash_list = PortfolioRepository::list_cash(pool, owner_id).await?;
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
            let cur_holdings: Vec<&Holding> = holdings.iter().filter(|h| h.currency == cur).collect();
            let cur_cash = cash_list.iter().find(|c| c.currency == cur);

            let mut holdings_value = 0i64;
            let mut exponent = 2i64;

            for h in cur_holdings {
                exponent = h.currency_exponent;
                if let Some(price) = h.current_price_minor {
                    holdings_value += (h.quantity_minor as i128 * price as i128 / 100_000_000) as i64;
                }
            }

            let cash_minor = cur_cash.map(|c| {
                exponent = c.currency_exponent;
                c.balance_minor
            }).unwrap_or(0);

            Self::record_snapshot(
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

    pub async fn get_history(
        pool: &SqlitePool,
        owner_id: &str,
        timeframe: &str,
        currency: &str,
    ) -> Result<PortfolioHistoryResponse> {
        let norm_currency = currency.trim().to_uppercase();
        let now = Utc::now();

        let days = match timeframe.to_lowercase().as_str() {
            "7d" => 7,
            "1m" => 30,
            "3m" => 90,
            "1y" => 365,
            _ => 3650, // "all" ~ 10 years
        };

        let cutoff_date = (now - Duration::days(days)).format("%Y-%m-%d").to_string();

        let mut snapshots = sqlx::query_as::<_, DailySnapshot>(
            r#"
            SELECT id, owner_id, snapshot_date, currency, currency_exponent,
                   holdings_value_minor, cash_minor, total_net_worth_minor, created_at
            FROM portfolio_daily_snapshots
            WHERE owner_id = ? AND currency = ? AND snapshot_date >= ?
            ORDER BY snapshot_date ASC;
            "#,
        )
        .bind(owner_id)
        .bind(&norm_currency)
        .bind(&cutoff_date)
        .fetch_all(pool)
        .await?;

        // Also fetch current live status to ensure today is always up to date
        let holdings = PortfolioRepository::list(pool, owner_id).await?;
        let cash_list = PortfolioRepository::list_cash(pool, owner_id).await?;

        let mut live_holdings_value = 0i64;
        let mut currency_exponent = 2i64;

        let cur_holdings: Vec<&Holding> = holdings.iter().filter(|h| h.currency == norm_currency).collect();

        for h in &cur_holdings {
            currency_exponent = h.currency_exponent;
            if let Some(price) = h.current_price_minor {
                live_holdings_value += (h.quantity_minor as i128 * price as i128 / 100_000_000) as i64;
            }
        }

        let live_cash = cash_list
            .iter()
            .find(|c| c.currency == norm_currency)
            .map(|c| {
                currency_exponent = c.currency_exponent;
                c.balance_minor
            })
            .unwrap_or(0);

        let live_total = live_holdings_value + live_cash;
        let today_str = now.format("%Y-%m-%d").to_string();

        // If user has active holdings, fetch their real historical price curves
        let mut holding_series: Vec<(&Holding, std::collections::BTreeMap<String, i64>)> = Vec::new();
        let mut all_dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for h in &cur_holdings {
            if let Ok(series) = crate::modules::portfolio::history_feed::fetch_historical_prices(&h.symbol, timeframe).await {
                let mut map = std::collections::BTreeMap::new();
                for (date, price) in series {
                    if let Some(created) = h.created_at {
                        let created_date = created.format("%Y-%m-%d").to_string();
                        if date < created_date {
                            continue;
                        }
                    }
                    let price_minor = crate::modules::portfolio::price_feed::price_to_minor(price, h.currency_exponent);
                    let val_minor = crate::modules::portfolio::models::scale_product(h.quantity_minor, price_minor);
                    all_dates.insert(date.clone());
                    map.insert(date, val_minor);
                }
                if !map.is_empty() {
                    holding_series.push((h, map));
                }
            }
        }

        let mut points: Vec<PortfolioHistoryPoint> = Vec::new();

        if !holding_series.is_empty() && !all_dates.is_empty() {
            // Forward-fill per holding so weekends or missing quotes don't create false dips
            let mut last_known_vals: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

            for date in all_dates {
                if date == today_str {
                    continue;
                }

                let mut total_holdings_for_date = 0i64;
                for (h, map) in &holding_series {
                    if let Some(&val) = map.get(&date) {
                        last_known_vals.insert(h.symbol.clone(), val);
                        total_holdings_for_date += val;
                    } else if let Some(&last_val) = last_known_vals.get(&h.symbol) {
                        total_holdings_for_date += last_val;
                    }
                }

                let date_cash = snapshots
                    .iter()
                    .find(|s| s.snapshot_date == date)
                    .map(|s| s.cash_minor)
                    .unwrap_or(live_cash);

                let total_net_worth = total_holdings_for_date + date_cash;

                // Cache/persist snapshot
                let _ = Self::record_snapshot(
                    pool,
                    owner_id,
                    &date,
                    &norm_currency,
                    currency_exponent,
                    total_holdings_for_date,
                    date_cash,
                )
                .await;

                points.push(PortfolioHistoryPoint {
                    date,
                    holdings_value_minor: total_holdings_for_date,
                    cash_minor: date_cash,
                    total_net_worth_minor: total_net_worth,
                });
            }
        } else {
            // Fall back to recorded snapshots
            points = snapshots
                .drain(..)
                .filter(|s| s.snapshot_date != today_str)
                .map(|s| PortfolioHistoryPoint {
                    date: s.snapshot_date,
                    holdings_value_minor: s.holdings_value_minor,
                    cash_minor: s.cash_minor,
                    total_net_worth_minor: s.total_net_worth_minor,
                })
                .collect();

            // If no prior snapshots exist, generate backfilled starting points from audit activity log
            if points.is_empty() {
                let audit_entries = sqlx::query_as::<_, (DateTime<Utc>, i64, String)>(
                    r#"
                    SELECT created_at, amount_minor, action
                    FROM portfolio_audit_log
                    WHERE owner_id = ? AND currency = ?
                    ORDER BY created_at ASC;
                    "#,
                )
                .bind(owner_id)
                .bind(&norm_currency)
                .fetch_all(pool)
                .await
                .unwrap_or_default();

                if !audit_entries.is_empty() {
                    let first_date = audit_entries[0].0.format("%Y-%m-%d").to_string();
                    if first_date != today_str {
                        points.push(PortfolioHistoryPoint {
                            date: first_date,
                            holdings_value_minor: 0,
                            cash_minor: audit_entries[0].1,
                            total_net_worth_minor: audit_entries[0].1,
                        });
                    }
                }
            }
        }

        // Add today's live point
        points.push(PortfolioHistoryPoint {
            date: today_str,
            holdings_value_minor: live_holdings_value,
            cash_minor: live_cash,
            total_net_worth_minor: live_total,
        });

        points.sort_by(|a, b| a.date.cmp(&b.date));
        points.dedup_by(|a, b| a.date == b.date);

        let starting_value_minor = points.first().map(|p| p.total_net_worth_minor).unwrap_or(0);
        let current_value_minor = points.last().map(|p| p.total_net_worth_minor).unwrap_or(0);
        let period_change_minor = current_value_minor - starting_value_minor;
        let period_change_pct = if starting_value_minor > 0 {
            (period_change_minor as f64 / starting_value_minor as f64) * 100.0
        } else {
            0.0
        };

        Ok(PortfolioHistoryResponse {
            currency: norm_currency,
            currency_exponent,
            timeframe: timeframe.to_string(),
            points,
            starting_value_minor,
            current_value_minor,
            period_change_minor,
            period_change_pct,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_history_snapshots_and_points() {
        let pool = pool().await;
        let owner = "bob";

        let today = chrono::Utc::now().date_naive();
        let d1 = (today - chrono::Duration::days(2)).format("%Y-%m-%d").to_string();
        let d2 = (today - chrono::Duration::days(1)).format("%Y-%m-%d").to_string();

        HistoryRepository::record_snapshot(
            &pool,
            owner,
            &d1,
            "USD",
            2i64,
            5000_00,
            1000_00,
        )
        .await
        .unwrap();

        HistoryRepository::record_snapshot(
            &pool,
            owner,
            &d2,
            "USD",
            2i64,
            5500_00,
            1000_00,
        )
        .await
        .unwrap();

        let hist = HistoryRepository::get_history(&pool, owner, "7d", "USD").await.unwrap();
        assert_eq!(hist.currency, "USD");
        assert!(hist.points.len() >= 2);
    }
}
