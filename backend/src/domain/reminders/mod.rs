//! Reminder domain entities, value objects and invariants.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

/// Maximum allowed length for a reminder message.
pub const MAX_MESSAGE_LEN: usize = 500;

/// Longest lead time accepted for due reminders (30 days).
pub const MAX_LEAD_MINUTES: i64 = 60 * 24 * 30;

/// Pattern for recurring reminders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecurrencePattern {
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl RecurrencePattern {
    pub fn as_str(&self) -> &'static str {
        match self {
            RecurrencePattern::None => "none",
            RecurrencePattern::Daily => "daily",
            RecurrencePattern::Weekly => "weekly",
            RecurrencePattern::Monthly => "monthly",
            RecurrencePattern::Yearly => "yearly",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "daily" | "day" => RecurrencePattern::Daily,
            "weekly" | "week" => RecurrencePattern::Weekly,
            "monthly" | "month" => RecurrencePattern::Monthly,
            "yearly" | "year" => RecurrencePattern::Yearly,
            _ => RecurrencePattern::None,
        }
    }
}

/// Status of a reminder in the lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReminderStatus {
    Pending,
    Triggered,
    Cancelled,
    Failed,
}

impl ReminderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderStatus::Pending => "pending",
            ReminderStatus::Triggered => "triggered",
            ReminderStatus::Cancelled => "cancelled",
            ReminderStatus::Failed => "failed",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "triggered" => ReminderStatus::Triggered,
            "cancelled" => ReminderStatus::Cancelled,
            "failed" => ReminderStatus::Failed,
            _ => ReminderStatus::Pending,
        }
    }
}

/// Which module's due date scheduled a reminder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderSource {
    Todo,
    RecurringBill,
}

impl ReminderSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderSource::Todo => "todo",
            ReminderSource::RecurringBill => "recurring_bill",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(ReminderSource::Todo),
            "recurring_bill" => Some(ReminderSource::RecurringBill),
            _ => None,
        }
    }
}

/// A reminder entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: i64,
    pub room_id: String,
    pub owner_id: AccountId,
    pub sender_name: String,
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub recurrence: RecurrencePattern,
    pub status: ReminderStatus,
    pub retry_count: i64,
    pub source_kind: Option<ReminderSource>,
    pub source_id: Option<i64>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Input data for creating a new reminder.
#[derive(Debug, Clone)]
pub struct NewReminder {
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub room_id: String,
    pub owner_name: String,
    pub recurrence: RecurrencePattern,
    pub source: Option<(ReminderSource, i64)>,
}

impl NewReminder {
    pub fn validate(&self, now: DateTime<Utc>) -> Result<(), &'static str> {
        validate_message(&self.message)?;
        validate_trigger_at(self.trigger_at, now)?;
        Ok(())
    }
}

/// What [`ReminderService::sync_due_reminder`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncOutcome {
    Nothing,
    Scheduled(DateTime<Utc>),
    Cancelled,
    LeadTimeInPast(DateTime<Utc>),
}

/// What an external feature wants reminding about.
#[derive(Debug, Clone)]
pub struct ReminderPlan {
    pub source: ReminderSource,
    pub source_id: i64,
    pub owner_id: AccountId,
    pub owner_name: String,
    pub room_id: String,
    pub message: String,
    pub trigger_at: Option<DateTime<Utc>>,
}

pub fn validate_message(message: &str) -> Result<(), &'static str> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Err("Message required");
    }
    if message.len() > MAX_MESSAGE_LEN {
        return Err("Message exceeds character limit");
    }
    Ok(())
}

pub fn validate_trigger_at(
    trigger_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<(), &'static str> {
    if trigger_at <= now {
        return Err("Trigger time must be in the future");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recurrence_pattern_conversion() {
        assert_eq!(
            RecurrencePattern::from_str_or_default("daily"),
            RecurrencePattern::Daily
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("day"),
            RecurrencePattern::Daily
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("weekly"),
            RecurrencePattern::Weekly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("week"),
            RecurrencePattern::Weekly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("monthly"),
            RecurrencePattern::Monthly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("month"),
            RecurrencePattern::Monthly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("yearly"),
            RecurrencePattern::Yearly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("year"),
            RecurrencePattern::Yearly
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("none"),
            RecurrencePattern::None
        );
        assert_eq!(
            RecurrencePattern::from_str_or_default("other"),
            RecurrencePattern::None
        );

        assert_eq!(RecurrencePattern::Daily.as_str(), "daily");
        assert_eq!(RecurrencePattern::Weekly.as_str(), "weekly");
        assert_eq!(RecurrencePattern::Monthly.as_str(), "monthly");
        assert_eq!(RecurrencePattern::Yearly.as_str(), "yearly");
        assert_eq!(RecurrencePattern::None.as_str(), "none");
    }

    #[test]
    fn test_reminder_status_conversion() {
        assert_eq!(
            ReminderStatus::from_str_or_default("pending"),
            ReminderStatus::Pending
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("triggered"),
            ReminderStatus::Triggered
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("cancelled"),
            ReminderStatus::Cancelled
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("failed"),
            ReminderStatus::Failed
        );
        assert_eq!(
            ReminderStatus::from_str_or_default("unknown"),
            ReminderStatus::Pending
        );

        assert_eq!(ReminderStatus::Pending.as_str(), "pending");
        assert_eq!(ReminderStatus::Triggered.as_str(), "triggered");
        assert_eq!(ReminderStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(ReminderStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn test_reminder_source_conversion() {
        assert_eq!(
            ReminderSource::from_str_opt("todo"),
            Some(ReminderSource::Todo)
        );
        assert_eq!(
            ReminderSource::from_str_opt("recurring_bill"),
            Some(ReminderSource::RecurringBill)
        );
        assert_eq!(ReminderSource::from_str_opt("unknown"), None);

        assert_eq!(ReminderSource::Todo.as_str(), "todo");
        assert_eq!(ReminderSource::RecurringBill.as_str(), "recurring_bill");
    }

    #[test]
    fn test_validation() {
        let now = Utc::now();
        assert!(validate_message("").is_err());
        assert!(validate_message("   ").is_err());
        assert!(validate_message(&"a".repeat(501)).is_err());
        assert!(validate_message("Valid reminder message").is_ok());

        assert!(validate_trigger_at(now - chrono::Duration::seconds(1), now).is_err());
        assert!(validate_trigger_at(now + chrono::Duration::seconds(10), now).is_ok());
    }
}
