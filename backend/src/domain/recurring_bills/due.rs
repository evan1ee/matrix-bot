use serde::{Deserialize, Serialize};

use crate::domain::recurring_bills::RecurringBill;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BillUrgency {
    DueToday,
    DueWithin3Days,
    DueWithin14Days,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BillDueAnalysis {
    pub bill_id: i64,
    pub name: String,
    pub due_date: String,
    pub amount_minor: i64,
    pub currency: String,
    pub urgency: BillUrgency,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CashFlowAdequacyReport {
    pub total_due_minor: i64,
    pub available_cash_minor: i64,
    pub projected_shortfall_minor: i64,
    pub bills: Vec<BillDueAnalysis>,
    pub has_shortfall: bool,
}

fn parse_due_date(raw: &str) -> Option<chrono::NaiveDate> {
    let trimmed = raw.trim();
    if trimmed.len() >= 10 {
        if let Ok(d) = chrono::NaiveDate::parse_from_str(&trimmed[..10], "%Y-%m-%d") {
            return Some(d);
        }
    }
    super::parse_due_at(raw).map(|dt| dt.date_naive())
}

pub fn evaluate_bill_cash_adequacy(
    bills: &[RecurringBill],
    available_cash_minor: i64,
    today: chrono::NaiveDate,
    days_ahead: i64,
) -> CashFlowAdequacyReport {
    let horizon = today + chrono::Duration::days(days_ahead);
    let mut analyzed_bills = Vec::new();
    let mut total_due_minor: i64 = 0;

    for b in bills {
        if !b.status.trim().eq_ignore_ascii_case("active") {
            continue;
        }

        let Some(due_date) = parse_due_date(&b.next_due_at) else {
            continue;
        };

        if due_date < today || due_date > horizon {
            continue;
        }

        let urgency = if due_date == today {
            BillUrgency::DueToday
        } else if due_date <= today + chrono::Duration::days(3) {
            BillUrgency::DueWithin3Days
        } else {
            BillUrgency::DueWithin14Days
        };

        total_due_minor += b.amount_minor;

        analyzed_bills.push(BillDueAnalysis {
            bill_id: b.id,
            name: b.name.clone(),
            due_date: due_date.format("%Y-%m-%d").to_string(),
            amount_minor: b.amount_minor,
            currency: b.currency.clone(),
            urgency,
        });
    }

    analyzed_bills.sort_by(|a, b| a.due_date.cmp(&b.due_date).then_with(|| a.bill_id.cmp(&b.bill_id)));

    let projected_shortfall_minor = total_due_minor.saturating_sub(available_cash_minor).max(0);
    let has_shortfall = projected_shortfall_minor > 0;

    CashFlowAdequacyReport {
        total_due_minor,
        available_cash_minor,
        projected_shortfall_minor,
        bills: analyzed_bills,
        has_shortfall,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use crate::domain::accounts::AccountId;

    fn sample_bill(id: i64, name: &str, due: &str, amount: i64, status: &str) -> RecurringBill {
        RecurringBill {
            id,
            room_id: "room1".to_string(),
            owner_id: AccountId::from_seed("user"),
            sender_name: "alice".to_string(),
            name: name.to_string(),
            category: "Utilities".to_string(),
            amount_minor: amount,
            currency: "USD".to_string(),
            currency_exponent: 2,
            frequency: "monthly".to_string(),
            due_day: Some(1),
            next_due_at: due.to_string(),
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
    fn test_urgency_grouping() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        let bills = vec![
            sample_bill(1, "Bill Today", "2026-10-18 10:00:00", 1_000, "active"),
            sample_bill(2, "Bill 2 Days", "2026-10-20", 2_000, "active"),
            sample_bill(3, "Bill 3 Days", "2026-10-21 12:00:00", 3_000, "active"),
            sample_bill(4, "Bill 7 Days", "2026-10-25", 4_000, "active"),
        ];

        let report = evaluate_bill_cash_adequacy(&bills, 50_000, today, 14);
        assert_eq!(report.bills.len(), 4);

        let b1 = report.bills.iter().find(|b| b.bill_id == 1).unwrap();
        assert_eq!(b1.urgency, BillUrgency::DueToday);

        let b2 = report.bills.iter().find(|b| b.bill_id == 2).unwrap();
        assert_eq!(b2.urgency, BillUrgency::DueWithin3Days);

        let b3 = report.bills.iter().find(|b| b.bill_id == 3).unwrap();
        assert_eq!(b3.urgency, BillUrgency::DueWithin3Days);

        let b4 = report.bills.iter().find(|b| b.bill_id == 4).unwrap();
        assert_eq!(b4.urgency, BillUrgency::DueWithin14Days);
    }

    #[test]
    fn test_horizon_and_status_filtering() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        let bills = vec![
            sample_bill(1, "Past Bill", "2026-10-17", 1_000, "active"), // in the past -> exclude
            sample_bill(2, "Valid Bill", "2026-10-22", 2_000, "active"), // within 14 days -> include
            sample_bill(3, "Far Future", "2026-11-05", 3_000, "active"), // beyond 14 days -> exclude
            sample_bill(4, "Paused Bill", "2026-10-20", 4_000, "paused"), // paused -> exclude
        ];

        let report = evaluate_bill_cash_adequacy(&bills, 10_000, today, 14);
        assert_eq!(report.bills.len(), 1);
        assert_eq!(report.bills[0].bill_id, 2);
        assert_eq!(report.total_due_minor, 2_000);
    }

    #[test]
    fn test_shortfall_detection() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        let bills = vec![
            sample_bill(1, "Bill A", "2026-10-19", 10_000, "active"),
            sample_bill(2, "Bill B", "2026-10-21", 5_000, "active"),
        ];

        // Total due: 15,000 minor ($150). Available: 10,000 minor ($100).
        let report = evaluate_bill_cash_adequacy(&bills, 10_000, today, 14);
        assert_eq!(report.total_due_minor, 15_000);
        assert_eq!(report.available_cash_minor, 10_000);
        assert_eq!(report.projected_shortfall_minor, 5_000);
        assert!(report.has_shortfall);
    }

    #[test]
    fn test_sufficient_cash() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 18).unwrap();
        let bills = vec![
            sample_bill(1, "Bill A", "2026-10-19", 10_000, "active"),
        ];

        // Total due: 10,000 minor ($100). Available: 20,000 minor ($200).
        let report = evaluate_bill_cash_adequacy(&bills, 20_000, today, 14);
        assert_eq!(report.total_due_minor, 10_000);
        assert_eq!(report.available_cash_minor, 20_000);
        assert_eq!(report.projected_shortfall_minor, 0);
        assert!(!report.has_shortfall);
    }
}
