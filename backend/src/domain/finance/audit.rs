use serde::{Deserialize, Serialize};

use crate::domain::finance::Transaction;
use crate::domain::recurring_bills::RecurringBill;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CategorySpendingAnalysis {
    pub category: String,
    pub total_minor: i64,
    pub transaction_count: usize,
    pub percentage_of_total: f64,
    pub is_spike: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MonthlyAuditReport {
    pub total_spending_minor: i64,
    pub recurring_bills_total_minor: i64,
    pub discretionary_spending_minor: i64,
    pub categories: Vec<CategorySpendingAnalysis>,
    pub flagged_spikes: Vec<String>,
}

pub fn evaluate_monthly_audit(
    transactions: &[Transaction],
    recurring_bills: &[RecurringBill],
    spike_threshold_pct: f64,
) -> MonthlyAuditReport {
    let mut total_spending_minor: i64 = 0;
    let mut category_stats: std::collections::BTreeMap<String, (i64, usize)> =
        std::collections::BTreeMap::new();

    for t in transactions {
        if t.direction == crate::domain::finance::Direction::Expense {
            total_spending_minor += t.amount_minor;
            let cat_name = t
                .category_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("Uncategorized")
                .to_string();

            let entry = category_stats.entry(cat_name).or_insert((0, 0));
            entry.0 += t.amount_minor;
            entry.1 += 1;
        }
    }

    let mut categories = Vec::with_capacity(category_stats.len());
    let mut flagged_spikes = Vec::new();

    for (category, (cat_total, count)) in category_stats {
        let percentage_of_total = if total_spending_minor > 0 {
            (cat_total as f64 / total_spending_minor as f64) * 100.0
        } else {
            0.0
        };
        let is_spike = total_spending_minor > 0 && percentage_of_total >= spike_threshold_pct;
        if is_spike {
            flagged_spikes.push(category.clone());
        }

        categories.push(CategorySpendingAnalysis {
            category,
            total_minor: cat_total,
            transaction_count: count,
            percentage_of_total,
            is_spike,
        });
    }

    // Sort categories by highest spend first, breaking ties alphabetically
    categories.sort_by(|a, b| {
        b.total_minor
            .cmp(&a.total_minor)
            .then_with(|| a.category.cmp(&b.category))
    });

    let recurring_bills_total_minor: i64 = recurring_bills
        .iter()
        .filter(|b| b.status.trim().eq_ignore_ascii_case("active"))
        .map(|b| b.amount_minor)
        .sum();

    let discretionary_spending_minor =
        total_spending_minor.saturating_sub(recurring_bills_total_minor).max(0);

    MonthlyAuditReport {
        total_spending_minor,
        recurring_bills_total_minor,
        discretionary_spending_minor,
        categories,
        flagged_spikes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use crate::domain::accounts::AccountId;
    use crate::domain::finance::Direction;

    fn sample_transaction(
        direction: Direction,
        amount_minor: i64,
        category_name: Option<&str>,
    ) -> Transaction {
        Transaction {
            id: 1,
            room_id: "room1".to_string(),
            owner_id: AccountId::from_seed("user"),
            sender_name: "alice".to_string(),
            direction,
            amount_minor,
            currency: "USD".to_string(),
            currency_exponent: 2,
            category_id: None,
            category_name: category_name.map(str::to_string),
            note: None,
            receipt_storage_key: None,
            source_kind: None,
            source_id: None,
            occurred_at: Utc::now(),
            created_at: None,
        }
    }

    fn sample_bill(amount_minor: i64, status: &str) -> RecurringBill {
        RecurringBill {
            id: 1,
            room_id: "room1".to_string(),
            owner_id: AccountId::from_seed("user"),
            sender_name: "alice".to_string(),
            name: "Test Bill".to_string(),
            category: "Utilities".to_string(),
            amount_minor,
            currency: "USD".to_string(),
            currency_exponent: 2,
            frequency: "monthly".to_string(),
            due_day: Some(1),
            next_due_at: "2026-11-01 00:00:00".to_string(),
            total_installments: None,
            paid_installments: 0,
            auto_pay: false,
            status: status.to_string(),
            notes: None,
            remind_before_minutes: None,
            remind_room_id: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn test_expenses_aggregation() {
        let transactions = vec![
            sample_transaction(Direction::Expense, 5_000, Some("Groceries")),
            sample_transaction(Direction::Expense, 3_000, Some("Groceries")),
            sample_transaction(Direction::Expense, 2_000, Some("Utilities")),
            sample_transaction(Direction::Income, 10_000, Some("Salary")),
        ];
        let recurring_bills = vec![];

        let report = evaluate_monthly_audit(&transactions, &recurring_bills, 50.0);
        assert_eq!(report.total_spending_minor, 10_000);
        assert_eq!(report.recurring_bills_total_minor, 0);
        assert_eq!(report.discretionary_spending_minor, 10_000);

        let groceries = report.categories.iter().find(|c| c.category == "Groceries").unwrap();
        assert_eq!(groceries.total_minor, 8_000);
        assert_eq!(groceries.transaction_count, 2);
        assert!((groceries.percentage_of_total - 80.0).abs() < 1e-4);

        let utilities = report.categories.iter().find(|c| c.category == "Utilities").unwrap();
        assert_eq!(utilities.total_minor, 2_000);
        assert_eq!(utilities.transaction_count, 1);
        assert!((utilities.percentage_of_total - 20.0).abs() < 1e-4);
    }

    #[test]
    fn test_recurring_bills_deduction() {
        let transactions = vec![
            sample_transaction(Direction::Expense, 50_000, Some("General")),
        ];
        let recurring_bills = vec![
            sample_bill(20_000, "active"),
            sample_bill(10_000, "paused"), // paused bill should NOT be counted
        ];

        let report = evaluate_monthly_audit(&transactions, &recurring_bills, 50.0);
        assert_eq!(report.total_spending_minor, 50_000);
        assert_eq!(report.recurring_bills_total_minor, 20_000);
        assert_eq!(report.discretionary_spending_minor, 30_000);
    }

    #[test]
    fn test_discretionary_spending_saturation() {
        let transactions = vec![
            sample_transaction(Direction::Expense, 10_000, Some("General")),
        ];
        let recurring_bills = vec![
            sample_bill(25_000, "active"),
        ];

        let report = evaluate_monthly_audit(&transactions, &recurring_bills, 50.0);
        assert_eq!(report.total_spending_minor, 10_000);
        assert_eq!(report.recurring_bills_total_minor, 25_000);
        assert_eq!(report.discretionary_spending_minor, 0);
    }

    #[test]
    fn test_spike_threshold_detection() {
        let transactions = vec![
            sample_transaction(Direction::Expense, 7_000, Some("Rent")),
            sample_transaction(Direction::Expense, 3_000, Some("Dining")),
        ];
        let recurring_bills = vec![];

        let report = evaluate_monthly_audit(&transactions, &recurring_bills, 50.0);
        let rent = report.categories.iter().find(|c| c.category == "Rent").unwrap();
        assert!(rent.is_spike);

        let dining = report.categories.iter().find(|c| c.category == "Dining").unwrap();
        assert!(!dining.is_spike);

        assert_eq!(report.flagged_spikes, vec!["Rent".to_string()]);
    }

    #[test]
    fn test_uncategorized_defaulting() {
        let transactions = vec![
            sample_transaction(Direction::Expense, 4_000, None),
            sample_transaction(Direction::Expense, 1_000, Some("")),
        ];
        let recurring_bills = vec![];

        let report = evaluate_monthly_audit(&transactions, &recurring_bills, 40.0);
        assert_eq!(report.categories.len(), 1);
        assert_eq!(report.categories[0].category, "Uncategorized");
        assert_eq!(report.categories[0].total_minor, 5_000);
        assert_eq!(report.categories[0].transaction_count, 2);
    }
}
