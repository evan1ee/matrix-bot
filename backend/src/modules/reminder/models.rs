use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Pattern for recurring reminders (none, daily, or weekly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum RecurrencePattern {
    None,
    Daily,
    Weekly,
}

impl RecurrencePattern {
    #[allow(dead_code)]
    pub fn as_str(&self) -> &'static str {
        match self {
            RecurrencePattern::None => "none",
            RecurrencePattern::Daily => "daily",
            RecurrencePattern::Weekly => "weekly",
        }
    }

    #[allow(dead_code)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "daily" | "day" => RecurrencePattern::Daily,
            "weekly" | "week" => RecurrencePattern::Weekly,
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
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "triggered" => ReminderStatus::Triggered,
            "cancelled" => ReminderStatus::Cancelled,
            "failed" => ReminderStatus::Failed,
            _ => ReminderStatus::Pending,
        }
    }
}

/// A reminder stored in the database.
#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Reminder {
    pub id: i64,
    pub room_id: String,
    pub sender_mxid: String,
    pub sender_name: String,
    pub message: String,
    pub trigger_at: DateTime<Utc>,
    pub recurrence: String,
    pub status: String,
    pub retry_count: i64,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_recurrence_pattern_conversion() {
        assert_eq!(RecurrencePattern::from_str("daily"), RecurrencePattern::Daily);
        assert_eq!(RecurrencePattern::from_str("day"), RecurrencePattern::Daily);
        assert_eq!(RecurrencePattern::from_str("weekly"), RecurrencePattern::Weekly);
        assert_eq!(RecurrencePattern::from_str("week"), RecurrencePattern::Weekly);
        assert_eq!(RecurrencePattern::from_str("none"), RecurrencePattern::None);
        assert_eq!(RecurrencePattern::from_str("random"), RecurrencePattern::None);

        assert_eq!(RecurrencePattern::Daily.as_str(), "daily");
        assert_eq!(RecurrencePattern::Weekly.as_str(), "weekly");
        assert_eq!(RecurrencePattern::None.as_str(), "none");
    }

    #[test]
    fn test_reminder_status_conversion() {
        assert_eq!(ReminderStatus::from_str("pending"), ReminderStatus::Pending);
        assert_eq!(ReminderStatus::from_str("triggered"), ReminderStatus::Triggered);
        assert_eq!(ReminderStatus::from_str("cancelled"), ReminderStatus::Cancelled);
        assert_eq!(ReminderStatus::from_str("failed"), ReminderStatus::Failed);
        assert_eq!(ReminderStatus::from_str("unknown"), ReminderStatus::Pending);

        assert_eq!(ReminderStatus::Pending.as_str(), "pending");
        assert_eq!(ReminderStatus::Triggered.as_str(), "triggered");
        assert_eq!(ReminderStatus::Cancelled.as_str(), "cancelled");
        assert_eq!(ReminderStatus::Failed.as_str(), "failed");
    }
}
