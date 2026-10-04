//! Domain model for account merging.

use serde::{Deserialize, Serialize};

/// How many rows each table would contribute to a merge.
///
/// Shown to the user before they confirm, because "this will move 47 todos and
/// 132 transactions into your account" is a decision someone should get to
/// make with the numbers in front of them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergePreview {
    pub todos: i64,
    pub reminders: i64,
    pub transactions: i64,
    pub recurring_bills: i64,
    pub portfolio_holdings: i64,
    pub portfolio_cash_balances: i64,
    pub finance_categories: i64,
    pub user_settings: i64,
    pub bookmarks: i64,
    pub habits: i64,
    pub habit_logs: i64,
    pub life_day_logs: i64,
    pub life_profiles: i64,
    pub footprints: i64,
    pub ai_conversations: i64,
    pub webhooks: i64,
    pub notes: i64,
    pub other: i64,
}

impl MergePreview {
    pub fn total(&self) -> i64 {
        self.todos
            + self.reminders
            + self.transactions
            + self.recurring_bills
            + self.portfolio_holdings
            + self.portfolio_cash_balances
            + self.finance_categories
            + self.user_settings
            + self.bookmarks
            + self.habits
            + self.habit_logs
            + self.life_day_logs
            + self.life_profiles
            + self.footprints
            + self.ai_conversations
            + self.webhooks
            + self.notes
            + self.other
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }

    pub fn set(&mut self, table: &str, count: i64) {
        match table {
            "todos" => self.todos = count,
            "reminders" => self.reminders = count,
            "transactions" => self.transactions = count,
            "recurring_bills" => self.recurring_bills = count,
            "portfolio_holdings" => self.portfolio_holdings = count,
            "portfolio_cash_balances" => self.portfolio_cash_balances = count,
            "finance_categories" => self.finance_categories = count,
            "user_settings" => self.user_settings = count,
            "bookmarks" => self.bookmarks = count,
            "habits" => self.habits = count,
            "habit_logs" => self.habit_logs = count,
            "life_day_logs" => self.life_day_logs = count,
            "life_profiles" => self.life_profiles = count,
            "footprints" => self.footprints = count,
            "ai_conversations" => self.ai_conversations = count,
            "webhooks" => self.webhooks = count,
            "notes" => self.notes = count,
            _ => self.other += count,
        }
    }
}

/// What a completed merge moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    pub moved: MergePreview,
    pub discarded: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_preview_total_and_set() {
        let mut preview = MergePreview::default();
        assert!(preview.is_empty());
        assert_eq!(preview.total(), 0);

        preview.set("todos", 5);
        preview.set("bookmarks", 3);
        preview.set("unknown_table", 2);

        assert_eq!(preview.todos, 5);
        assert_eq!(preview.bookmarks, 3);
        assert_eq!(preview.other, 2);
        assert_eq!(preview.total(), 10);
        assert!(!preview.is_empty());
    }
}
