use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioHistoryPoint {
    pub date: String,
    pub holdings_value_minor: i64,
    pub cash_minor: i64,
    pub total_net_worth_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
