use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Pattern for recurring reminders (none, daily, weekly, monthly, or yearly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RecurrencePattern {
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl RecurrencePattern {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            RecurrencePattern::None => "none",
            RecurrencePattern::Daily => "daily",
            RecurrencePattern::Weekly => "weekly",
            RecurrencePattern::Monthly => "monthly",
            RecurrencePattern::Yearly => "yearly",
        }
    }

    #[allow(dead_code)]
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

/// Status of a reminder in the database.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum ReminderStatus {
    Pending,
    Triggered,
    Cancelled,
    Failed,
}

impl ReminderStatus {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderStatus::Pending => "pending",
            ReminderStatus::Triggered => "triggered",
            ReminderStatus::Cancelled => "cancelled",
            ReminderStatus::Failed => "failed",
        }
    }

    #[allow(dead_code)]
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
///
/// Reminders created directly (`!remind`, the REST create) carry no source; a
/// source is what lets a due date change reschedule or cancel the reminder it
/// produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReminderSource {
    Todo,
    RecurringBill,
}

impl ReminderSource {
    /// The value stored in `reminders.source_kind`.
    pub fn as_str(&self) -> &'static str {
        match self {
            ReminderSource::Todo => "todo",
            ReminderSource::RecurringBill => "recurring_bill",
        }
    }
}

/// A reminder stored in the database.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Reminder {
    pub id: i64,
    pub room_id: String,
    pub owner_id: String,
    pub sender_name: String,
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub recurrence: String,
    pub status: String,
    pub retry_count: i64,
    /// Which module scheduled this reminder, if any — see [`ReminderSource`].
    pub source_kind: Option<String>,
    /// The id of the row in that module which scheduled this reminder, so
    /// changing or retiring that row can reschedule or cancel it.
    pub source_id: Option<i64>,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recurrence_pattern_conversion() {
        assert_eq!(RecurrencePattern::from_str_or_default("daily"), RecurrencePattern::Daily);
        assert_eq!(RecurrencePattern::from_str_or_default("day"), RecurrencePattern::Daily);
        assert_eq!(RecurrencePattern::from_str_or_default("weekly"), RecurrencePattern::Weekly);
        assert_eq!(RecurrencePattern::from_str_or_default("week"), RecurrencePattern::Weekly);
        assert_eq!(RecurrencePattern::from_str_or_default("monthly"), RecurrencePattern::Monthly);
        assert_eq!(RecurrencePattern::from_str_or_default("month"), RecurrencePattern::Monthly);
        assert_eq!(RecurrencePattern::from_str_or_default("yearly"), RecurrencePattern::Yearly);
        assert_eq!(RecurrencePattern::from_str_or_default("year"), RecurrencePattern::Yearly);
        assert_eq!(RecurrencePattern::from_str_or_default("none"), RecurrencePattern::None);
        assert_eq!(RecurrencePattern::from_str_or_default("random"), RecurrencePattern::None);

        assert_eq!(RecurrencePattern::Daily.as_str(), "daily");
        assert_eq!(RecurrencePattern::Weekly.as_str(), "weekly");
        assert_eq!(RecurrencePattern::Monthly.as_str(), "monthly");
        assert_eq!(RecurrencePattern::Yearly.as_str(), "yearly");
        assert_eq!(RecurrencePattern::None.as_str(), "none");
    }

    #[test]
    fn test_reminder_status_conversion() {
        assert_eq!(ReminderStatus::from_str_or_default("pending"), ReminderStatus::Pending);
        assert_eq!(ReminderStatus::from_str_or_default("triggered"), ReminderStatus::Triggered);
        assert_eq!(ReminderStatus::from_str_or_default("cancelled"), ReminderStatus::Cancelled);
        assert_eq!(ReminderStatus::from_str_or_default("failed"), ReminderStatus::Failed);
        assert_eq!(ReminderStatus::from_str_or_default("unknown"), ReminderStatus::Pending);

        assert_eq!(ReminderStatus::Pending.as_str(), "pending");
        assert_eq!(ReminderStatus::Triggered.as_str(), "triggered");
        assert_eq!(ReminderStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(ReminderStatus::Failed.as_str(), "failed");
    }
}
