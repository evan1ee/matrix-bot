//! Pure domain types for net worth calculations across finance and portfolio.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
