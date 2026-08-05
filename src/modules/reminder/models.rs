use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecurrencePattern {
    None,
    Daily,
    Weekly,
}

impl RecurrencePattern {
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

#[allow(dead_code)]
#[derive(Debug, Clone, sqlx::FromRow)]
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
}
