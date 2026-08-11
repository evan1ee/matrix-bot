use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Which way the money moved.
///
/// `amount_minor` is always positive; this is what carries the sign, so a row
/// cannot contradict itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
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
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Category {
    pub id: i64,
    /// The owning `owner_id`. Categories are per-user, never shared.
    pub owner_id: String,
    pub name: String,
    pub created_at: Option<DateTime<Utc>>,
}

/// A single ledger entry.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Transaction {
    pub id: i64,
    pub room_id: String,
    /// The sole ownership key, matching `AuthUser::owner_id()`.
    pub owner_id: String,
    /// Display only — user-controlled, and collides across identity spaces.
    pub sender_name: String,
    pub direction: String,
    /// Minor units (cents), always positive.
    pub amount_minor: i64,
    pub currency: String,
    /// Decimal places for this row's currency, stored so a later change to the
    /// currency table cannot rewrite what this row means.
    pub currency_exponent: i64,
    /// `None` means uncategorised, which is a valid and expected state.
    pub category_id: Option<i64>,
    /// Denormalised for reads so listing a ledger is one query, not N+1.
    #[sqlx(default)]
    pub category_name: Option<String>,
    pub note: Option<String>,
    /// When the money moved — not when the row was written.
    pub occurred_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Per-category totals within one currency.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CategoryTotal {
    /// `None` for uncategorised entries.
    pub category: Option<String>,
    pub expense_minor: i64,
    pub income_minor: i64,
}

/// Totals for one currency. Currencies are never combined — without a rate
/// source, adding EUR to JPY would produce a meaningless number.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CurrencySummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub expense_minor: i64,
    pub income_minor: i64,
    /// `income - expense`. May legitimately be negative.
    pub net_minor: i64,
    pub categories: Vec<CategoryTotal>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_direction_round_trip() {
        assert_eq!(Direction::parse("expense"), Some(Direction::Expense));
        assert_eq!(Direction::parse("INCOME"), Some(Direction::Income));
        assert_eq!(Direction::parse(" in "), Some(Direction::Income));
        assert_eq!(Direction::Expense.as_str(), "expense");
        assert_eq!(Direction::Income.as_str(), "income");
    }

    /// An unrecognised direction must fail rather than defaulting, or a typo
    /// silently books income as spending.
    #[test]
    fn test_unknown_direction_is_rejected() {
        assert_eq!(Direction::parse("transfer"), None);
        assert_eq!(Direction::parse(""), None);
    }
}
