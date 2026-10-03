use chrono::{Duration, Utc};

use super::PortfolioService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::history::{PortfolioHistoryPoint, PortfolioHistoryResponse};
use crate::domain::portfolio::models::{scale_product, Holding};
use crate::infrastructure::postgres::repositories::portfolio::audit as audit_repo;
use crate::infrastructure::postgres::repositories::portfolio::cash as cash_repo;
use crate::infrastructure::postgres::repositories::portfolio::history as history_repo;
use crate::infrastructure::postgres::repositories::portfolio::holdings as holdings_repo;
use crate::infrastructure::web::portfolio_feed::price_to_minor;
use crate::infrastructure::web::portfolio_history_feed::fetch_historical_prices;

impl PortfolioService {
    pub async fn record_daily_snapshot(
        &self,
        owner: &AccountId,
        snapshot_date: &str,
        currency: &str,
        currency_exponent: i64,
        holdings_value_minor: i64,
        cash_minor: i64,
    ) -> Result<(), AppError> {
        history_repo::record_snapshot(
            &self.pool,
            owner,
            snapshot_date,
            currency,
            currency_exponent,
            holdings_value_minor,
            cash_minor,
        )
        .await?;
        Ok(())
    }

    pub async fn capture_current_snapshot_for_owner(
        &self,
        owner: &AccountId,
    ) -> Result<(), AppError> {
        history_repo::capture_current_snapshot_for_owner(&self.pool, owner).await?;
        Ok(())
    }

    pub async fn get_history(
        &self,
        owner: &AccountId,
        timeframe: &str,
        currency: &str,
    ) -> Result<PortfolioHistoryResponse, AppError> {
        let norm_currency = currency.trim().to_uppercase();
        let now = Utc::now();

        let days = match timeframe.to_lowercase().as_str() {
            "7d" => 7,
            "1m" => 30,
            "3m" => 90,
            "1y" => 365,
            _ => 3650,
        };

        let cutoff_date = (now - Duration::days(days)).format("%Y-%m-%d").to_string();

        let mut snapshots =
            history_repo::get_snapshots(&self.pool, owner, &norm_currency, &cutoff_date).await?;

        let holdings = holdings_repo::list(&self.pool, owner).await?;
        let cash_list = cash_repo::list_cash(&self.pool, owner).await?;

        let mut live_holdings_value = 0i64;
        let mut currency_exponent = 2i64;

        let cur_holdings: Vec<&Holding> = holdings
            .iter()
            .filter(|h| h.currency == norm_currency)
            .collect();

        for h in &cur_holdings {
            currency_exponent = h.currency_exponent;
            if let Some(price) = h.current_price_minor {
                live_holdings_value += scale_product(h.quantity_minor, price);
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

        let mut holding_series: Vec<(&Holding, std::collections::BTreeMap<String, i64>)> =
            Vec::new();
        let mut all_dates: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();

        for h in &cur_holdings {
            if let Ok(series) = fetch_historical_prices(&h.symbol, timeframe).await {
                let mut map = std::collections::BTreeMap::new();
                for (date, price) in series {
                    if let Some(created) = h.created_at {
                        let created_date = created.format("%Y-%m-%d").to_string();
                        if date < created_date {
                            continue;
                        }
                    }
                    let price_minor = price_to_minor(price, h.currency_exponent);
                    let val_minor = scale_product(h.quantity_minor, price_minor);
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
            let mut last_known_vals: std::collections::HashMap<String, i64> =
                std::collections::HashMap::new();

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

                let _ = history_repo::record_snapshot(
                    &self.pool,
                    owner,
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

            if points.is_empty() {
                if let Some((created, amount_minor)) =
                    audit_repo::first_audit_entry_for_currency(&self.pool, owner, &norm_currency)
                        .await?
                {
                    let first_date = created.format("%Y-%m-%d").to_string();
                    if first_date != today_str {
                        points.push(PortfolioHistoryPoint {
                            date: first_date,
                            holdings_value_minor: 0,
                            cash_minor: amount_minor,
                            total_net_worth_minor: amount_minor,
                        });
                    }
                }
            }
        }

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
