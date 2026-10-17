//! Recurring bills domain entities, value objects and invariants.

pub mod due;

use chrono::{DateTime, Duration, Months, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

pub use due::{evaluate_bill_cash_adequacy, BillDueAnalysis, BillUrgency, CashFlowAdequacyReport};

use crate::domain::accounts::AccountId;

/// Maximum allowed length for a bill name or note.
pub const MAX_BILL_TEXT_LEN: usize = 500;

/// Represents a recurring bill, subscription or installment plan entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringBill {
    pub id: i64,
    pub room_id: String,
    pub owner_id: AccountId,
    pub sender_name: String,
    pub name: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub frequency: String,
    pub due_day: Option<i32>,
    pub next_due_at: String,
    pub total_installments: Option<i32>,
    pub paid_installments: i32,
    pub auto_pay: bool,
    pub status: String,
    pub notes: Option<String>,
    pub remind_before_minutes: Option<i64>,
    pub remind_room_id: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// Input data for creating a new recurring bill.
#[derive(Debug, Clone)]
pub struct NewRecurringBill {
    pub room_id: String,
    pub sender_name: String,
    pub name: String,
    pub category: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub frequency: String,
    pub due_day: Option<i32>,
    pub next_due_at: String,
    pub total_installments: Option<i32>,
    pub auto_pay: bool,
    pub notes: Option<String>,
    pub remind_before_minutes: Option<i64>,
    pub remind_room_id: Option<String>,
}

impl NewRecurringBill {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() {
            return Err("Bill name cannot be empty");
        }
        if self.name.len() > MAX_BILL_TEXT_LEN {
            return Err("Bill name exceeds maximum length");
        }
        if self.category.trim().is_empty() {
            return Err("Bill category cannot be empty");
        }
        if self.amount_minor <= 0 {
            return Err("Bill amount must be positive");
        }
        if self.currency.trim().is_empty() {
            return Err("Bill currency cannot be empty");
        }
        if parse_due_at(&self.next_due_at).is_none() {
            return Err("Invalid next due date format");
        }
        if let Some(m) = self.remind_before_minutes {
            if m <= 0 {
                return Err("Reminder lead time must be positive");
            }
            if m > crate::domain::reminders::MAX_LEAD_MINUTES {
                return Err("Reminder lead time exceeds maximum allowed");
            }
        }
        Ok(())
    }
}

/// A patch to update an existing recurring bill.
#[derive(Debug, Clone, Default)]
pub struct RecurringBillPatch {
    pub name: Option<String>,
    pub category: Option<String>,
    pub amount_minor: Option<i64>,
    pub currency: Option<String>,
    pub frequency: Option<String>,
    pub due_day: Option<i32>,
    pub next_due_at: Option<String>,
    pub total_installments: Option<i32>,
    pub paid_installments: Option<i32>,
    pub auto_pay: Option<bool>,
    pub status: Option<String>,
    pub notes: Option<String>,
    pub remind_before_minutes: Option<Option<i64>>,
    pub remind_room_id: Option<String>,
}

impl RecurringBillPatch {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(ref name) = self.name {
            if name.trim().is_empty() {
                return Err("Bill name cannot be empty");
            }
            if name.len() > MAX_BILL_TEXT_LEN {
                return Err("Bill name exceeds maximum length");
            }
        }
        if let Some(amount) = self.amount_minor {
            if amount <= 0 {
                return Err("Bill amount must be positive");
            }
        }
        if let Some(ref due) = self.next_due_at {
            if parse_due_at(due).is_none() {
                return Err("Invalid next due date format");
            }
        }
        if let Some(Some(m)) = self.remind_before_minutes {
            if m <= 0 {
                return Err("Reminder lead time must be positive");
            }
            if m > crate::domain::reminders::MAX_LEAD_MINUTES {
                return Err("Reminder lead time exceeds maximum allowed");
            }
        }
        Ok(())
    }
}

/// Advances a bill's due date string based on its frequency, anchoring
/// month-based frequencies (`monthly`, `quarterly`, `semiannually`, `yearly`)
/// to `due_day` (clamped to the target month's last calendar day) when provided.
pub fn advance_due_date_with_due_day(
    current_due_at: &str,
    frequency: &str,
    due_day: Option<i32>,
) -> String {
    let dt = parse_due_at(current_due_at).unwrap_or_else(Utc::now);
    let freq = frequency.trim().to_ascii_lowercase();

    let next_dt = match freq.as_str() {
        "daily" => dt + Duration::days(1),
        "weekly" => dt + Duration::weeks(1),
        "biweekly" => dt + Duration::weeks(2),
        "monthly" => add_months_with_anchor(dt, 1, due_day),
        "quarterly" => add_months_with_anchor(dt, 3, due_day),
        "semiannually" => add_months_with_anchor(dt, 6, due_day),
        "yearly" => add_months_with_anchor(dt, 12, due_day),
        _ => add_months_with_anchor(dt, 1, due_day),
    };

    next_dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn add_months_with_anchor(dt: DateTime<Utc>, months: u32, due_day: Option<i32>) -> DateTime<Utc> {
    use chrono::Datelike;
    let stepped = dt
        .checked_add_months(Months::new(months))
        .unwrap_or_else(|| dt + Duration::days(i64::from(months) * 30));
    let Some(anchor) = due_day.filter(|&d| (1..=31).contains(&d)) else {
        return stepped;
    };
    let year = stepped.year();
    let month = stepped.month();
    let last_day = last_day_of_month(year, month);
    let target_day = (anchor as u32).min(last_day);
    stepped.with_day(target_day).unwrap_or(stepped)
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    let (next_year, next_month) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    chrono::NaiveDate::from_ymd_opt(next_year, next_month, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| chrono::Datelike::day(&d))
        .unwrap_or(28)
}

/// Advances a bill's due date string based on its frequency.
pub fn advance_due_date(current_due_at: &str, frequency: &str) -> String {
    advance_due_date_with_due_day(current_due_at, frequency, None)
}

/// Parses a stored `next_due_at`, which may be one of several text formats.
pub fn parse_due_at(raw: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw.trim()) {
        return Some(dt.with_timezone(&Utc));
    }
    NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S"))
        .or_else(|_| NaiveDateTime::parse_from_str(&format!("{raw} 00:00:00"), "%Y-%m-%d %H:%M:%S"))
        .ok()
        .map(|ndt| Utc.from_utc_datetime(&ndt))
}

/// Whether a bill in this status still has periods left to remind about.
pub fn still_recurring(status: &str) -> bool {
    status.eq_ignore_ascii_case("active")
}

/// The instant a reminder should fire for `bill`, if it wants one at all.
pub fn wanted_trigger(bill: &RecurringBill) -> Option<DateTime<Utc>> {
    let minutes = bill.remind_before_minutes?;
    if minutes <= 0 || !still_recurring(&bill.status) {
        return None;
    }
    Some(parse_due_at(&bill.next_due_at)? - Duration::minutes(minutes))
}

/// Where the bill's reminder should be delivered.
pub fn reminder_room(bill: &RecurringBill) -> Option<&str> {
    match bill.remind_room_id.as_deref() {
        Some(room) if !room.trim().is_empty() => Some(room),
        _ if bill.room_id != "web" => Some(&bill.room_id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advance_due_date_frequencies() {
        let base = "2026-01-15 10:00:00";
        assert_eq!(advance_due_date(base, "daily"), "2026-01-16 10:00:00");
        assert_eq!(advance_due_date(base, "weekly"), "2026-01-22 10:00:00");
        assert_eq!(advance_due_date(base, "biweekly"), "2026-01-29 10:00:00");
        assert_eq!(advance_due_date(base, "monthly"), "2026-02-15 10:00:00");
        assert_eq!(advance_due_date(base, "yearly"), "2027-01-15 10:00:00");

        // M1: A bill with due_day=31 clamped to Feb 28 advances to Mar 31, not Mar 28
        let feb = advance_due_date_with_due_day("2026-01-31 10:00:00", "monthly", Some(31));
        assert_eq!(feb, "2026-02-28 10:00:00");
        let mar = advance_due_date_with_due_day(&feb, "monthly", Some(31));
        assert_eq!(mar, "2026-03-31 10:00:00");
    }

    #[test]
    fn test_parse_due_at() {
        assert!(parse_due_at("2026-05-01 12:00:00").is_some());
        assert!(parse_due_at("2026-05-01T12:00:00").is_some());
        assert!(parse_due_at("2026-05-01").is_some());
        assert!(parse_due_at("invalid").is_none());
    }

    #[test]
    fn test_wanted_trigger() {
        let bill = RecurringBill {
            id: 42,
            room_id: "!room:test".to_string(),
            owner_id: AccountId::from_seed("acct_alice"),
            sender_name: "alice".to_string(),
            name: "Netflix".to_string(),
            category: "Entertainment".to_string(),
            amount_minor: 1599,
            currency: "USD".to_string(),
            currency_exponent: 2,
            frequency: "monthly".to_string(),
            due_day: Some(15),
            next_due_at: "2026-05-15 12:00:00".to_string(),
            total_installments: None,
            paid_installments: 3,
            auto_pay: false,
            status: "active".to_string(),
            notes: None,
            remind_before_minutes: Some(120),
            remind_room_id: None,
            created_at: None,
            updated_at: None,
        };

        let trigger = wanted_trigger(&bill).unwrap();
        let expected = parse_due_at("2026-05-15 12:00:00").unwrap() - Duration::minutes(120);
        assert_eq!(trigger, expected);

        let mut paused = bill.clone();
        paused.status = "paused".to_string();
        assert_eq!(wanted_trigger(&paused), None);
    }
}
