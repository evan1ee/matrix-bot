//! Pure domain types for financial analytics and public reports.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonthlyCashflowBucket {
    pub month: String, // "YYYY-MM"
    pub income_minor: i64,
    pub expense_minor: i64,
    pub net_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpenseCategorySlice {
    pub category_name: String,
    pub amount_minor: i64,
    pub percentage: f64,
    pub tx_count: i64,
    pub currency: String,
    pub currency_exponent: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FinanceAnalyticsReport {
    pub currency: String,
    pub currency_exponent: i64,
    pub months_requested: i64,
    pub total_income_minor: i64,
    pub total_expense_minor: i64,
    pub net_savings_minor: i64,
    pub monthly_cashflow: Vec<MonthlyCashflowBucket>,
    pub expense_categories: Vec<ExpenseCategorySlice>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicTransaction {
    pub id: i64,
    pub direction: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub category_name: Option<String>,
    pub note: Option<String>,
    pub occurred_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicFinanceReport {
    pub title: String,
    pub currency: String,
    pub currency_exponent: i64,
    pub total_income_minor: i64,
    pub total_expense_minor: i64,
    pub net_savings_minor: i64,
    pub monthly_cashflow: Vec<MonthlyCashflowBucket>,
    pub expense_categories: Vec<ExpenseCategorySlice>,
    pub recent_transactions: Vec<PublicTransaction>,
}
