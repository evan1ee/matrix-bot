use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum TodoPriority {
    Low,
    Medium,
    High,
}

impl TodoPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            TodoPriority::Low => "low",
            TodoPriority::Medium => "medium",
            TodoPriority::High => "high",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "high" | "h" => TodoPriority::High,
            "low" | "l" => TodoPriority::Low,
            _ => TodoPriority::Medium,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct TodoItem {
    pub id: i64,
    pub room_id: String,
    pub owner_id: String,
    pub sender_name: String,
    pub text: String,
    pub priority: String,
    /// When the task is due, stored as a real timestamp so a lead-time reminder
    /// can be computed from it. Rendered for display only at the edges.
    pub due_at: Option<DateTime<Utc>>,
    /// Minutes before `due_at` to fire a reminder. `None` means no reminder.
    pub remind_before_minutes: Option<i64>,
    pub completed: bool,
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_priority_conversion() {
        assert_eq!(TodoPriority::from_str("high"), TodoPriority::High);
        assert_eq!(TodoPriority::from_str("h"), TodoPriority::High);
        assert_eq!(TodoPriority::from_str("low"), TodoPriority::Low);
        assert_eq!(TodoPriority::from_str("l"), TodoPriority::Low);
        assert_eq!(TodoPriority::from_str("medium"), TodoPriority::Medium);
        assert_eq!(TodoPriority::from_str("anything_else"), TodoPriority::Medium);

        assert_eq!(TodoPriority::High.as_str(), "high");
        assert_eq!(TodoPriority::Medium.as_str(), "medium");
        assert_eq!(TodoPriority::Low.as_str(), "low");
    }
}
