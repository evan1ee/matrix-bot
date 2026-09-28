//! Finance domain entities, value objects, and business invariants.
//!
//! Encapsulates transactions, categories, spending summaries, money handling,
//! and sharing without dependencies on transports or database frameworks.

pub mod analytics;
pub mod fx;
pub mod money;
pub mod net_worth;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

/// Maximum allowed length for a transaction's note.
pub const MAX_NOTE_LEN: usize = 500;
/// Maximum allowed length for a category name.
pub const MAX_CATEGORY_LEN: usize = 64;

/// Which way the money moved.
///
/// `amount_minor` is always positive; this is what carries the sign, so a row
/// cannot contradict itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Expense,
    Income,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Expense => "expense",
            Direction::Income => "income",
        }
    }

    /// Parses a direction, returning `None` rather than guessing — an unknown
    /// word must not silently become an expense.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "expense" | "expenses" | "spend" | "out" => Some(Direction::Expense),
            "income" | "in" | "earn" => Some(Direction::Income),
            _ => None,
        }
    }
}

/// A user-defined spending category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Category {
    pub id: i64,
    /// The owning account ID. Categories are per-user, never shared.
    pub owner_id: AccountId,
    pub name: String,
    pub created_at: Option<DateTime<Utc>>,
}

/// A single ledger entry entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transaction {
    pub id: i64,
    pub room_id: String,
    pub owner_id: AccountId,
    pub sender_name: String,
    pub direction: Direction,
    /// Minor units (cents), always positive.
    pub amount_minor: i64,
    pub currency: String,
    /// Decimal places for this row's currency, stored so a later change to the
    /// currency table cannot rewrite what this row means.
    pub currency_exponent: i64,
    /// `None` means uncategorised, which is a valid and expected state.
    pub category_id: Option<i64>,
    /// Denormalised for reads so listing a ledger is one query, not N+1.
    pub category_name: Option<String>,
    pub note: Option<String>,
    /// Which module scheduled or generated this transaction (e.g. "recurring_bill", "portfolio_cash").
    pub source_kind: Option<String>,
    /// The ID of the originating entity in that module.
    pub source_id: Option<i64>,
    /// When the money moved — not when the row was written.
    pub occurred_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Known automated sources for transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionSource {
    RecurringBill,
    PortfolioCash,
}

impl TransactionSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransactionSource::RecurringBill => "recurring_bill",
            TransactionSource::PortfolioCash => "portfolio_cash",
        }
    }
}

/// Per-category totals within one currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoryTotal {
    /// `None` for uncategorised entries.
    pub category: Option<String>,
    pub expense_minor: i64,
    pub income_minor: i64,
}

/// Totals for one currency. Currencies are never combined without an explicit exchange rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrencySummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub expense_minor: i64,
    pub income_minor: i64,
    /// `income - expense`. May legitimately be negative.
    pub net_minor: i64,
    pub categories: Vec<CategoryTotal>,
}

/// A read-only share token for public analytics viewing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinanceShare {
    pub owner_id: AccountId,
    pub token: String,
    pub is_enabled: bool,
    pub title: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Input parameters for creating a new transaction.
#[derive(Debug, Clone)]
pub struct NewTransaction<'a> {
    pub direction: Direction,
    pub amount_minor: i64,
    pub currency: &'a str,
    pub currency_exponent: i64,
    pub category: Option<&'a str>,
    pub note: Option<&'a str>,
    pub room_id: &'a str,
    pub sender_name: &'a str,
    pub occurred_at: DateTime<Utc>,
    pub source: Option<(&'a str, i64)>,
    pub create_missing_category: bool,
}

/// Fields to change on an existing transaction.
#[derive(Debug, Default, Clone)]
pub struct TransactionPatch<'a> {
    pub direction: Option<Direction>,
    pub amount_minor: Option<i64>,
    pub currency: Option<&'a str>,
    pub currency_exponent: Option<i64>,
    pub category: Option<Option<&'a str>>,
    pub note: Option<Option<&'a str>>,
    pub occurred_at: Option<DateTime<Utc>>,
}

/// Filter criteria for listing transactions.
#[derive(Debug, Default, Clone)]
pub struct ListTransactionsFilter<'a> {
    pub direction: Option<Direction>,
    pub category_id: Option<i64>,
    pub since: Option<DateTime<Utc>>,
    pub until: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub currency: Option<&'a str>,
}

/// Domain errors encountered during finance operations.
#[derive(Debug, thiserror::Error)]
pub enum FinanceError {
    #[error("Note exceeds {max} character limit")]
    NoteTooLong { max: usize },

    #[error("Category name must be 1\u{2013}{max} characters")]
    CategoryNameInvalid { max: usize },

    #[error("You already have a category with that name")]
    CategoryExists,

    #[error("Entry not found or access denied")]
    TransactionNotFound,

    #[error("Category not found or access denied")]
    CategoryNotFound,

    #[error("No category named {name}")]
    UnknownCategory { name: String },

    #[error("Share token not found or disabled")]
    ShareNotFound,

    #[error("Invalid money format: {0}")]
    Money(#[from] money::MoneyError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_parsing() {
        assert_eq!(Direction::parse("expense"), Some(Direction::Expense));
        assert_eq!(Direction::parse("EXPENSE"), Some(Direction::Expense));
        assert_eq!(Direction::parse("spend"), Some(Direction::Expense));
        assert_eq!(Direction::parse("out"), Some(Direction::Expense));
        assert_eq!(Direction::parse("income"), Some(Direction::Income));
        assert_eq!(Direction::parse("in"), Some(Direction::Income));
        assert_eq!(Direction::parse("earn"), Some(Direction::Income));
        assert_eq!(Direction::parse("invalid"), None);
    }
}
