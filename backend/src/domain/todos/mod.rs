//! Todo domain entities, value objects and invariants.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

/// Maximum allowed length for a todo description.
pub const MAX_TEXT_LEN: usize = 500;

/// Priority level of a todo task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

    pub fn from_str_or_default(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "high" | "h" => TodoPriority::High,
            "low" | "l" => TodoPriority::Low,
            _ => TodoPriority::Medium,
        }
    }
}

/// A todo task item entity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub id: i64,
    pub room_id: String,
    pub owner_id: AccountId,
    pub sender_name: String,
    pub text: String,
    pub priority: TodoPriority,
    pub due_at: Option<DateTime<Utc>>,
    pub remind_before_minutes: Option<i64>,
    pub completed: bool,
    pub created_at: Option<DateTime<Utc>>,
}

/// Input data for creating a new todo item.
#[derive(Debug, Clone)]
pub struct NewTodo {
    pub room_id: String,
    pub sender_name: String,
    pub text: String,
    pub priority: TodoPriority,
    pub due_at: Option<DateTime<Utc>>,
    pub remind_before_minutes: Option<i64>,
    pub remind_room_id: Option<String>,
}

impl NewTodo {
    pub fn validate(&self) -> Result<(), &'static str> {
        validate_text(&self.text)?;
        if self.remind_before_minutes.is_some() && self.due_at.is_none() {
            return Err("remind_before_minutes requires a due_at to count back from");
        }
        if let Some(m) = self.remind_before_minutes {
            if m <= 0 || m > crate::domain::reminders::MAX_LEAD_MINUTES {
                return Err(
                    "remind_before_minutes must be greater than zero and at most 43200 (30 days)",
                );
            }
            let has_room = match self.remind_room_id.as_deref() {
                Some(r) => !r.trim().is_empty(),
                None => self.room_id != "web" && !self.room_id.trim().is_empty(),
            };
            if !has_room {
                return Err(
                    "remind_room_id is required for a reminder on a task that has no Matrix room",
                );
            }
        }
        Ok(())
    }
}

/// A patch to update an existing todo.
#[derive(Debug, Clone, Default)]
pub struct TodoPatch {
    pub text: Option<String>,
    pub priority: Option<TodoPriority>,
    pub completed: Option<bool>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub remind_before_minutes: Option<Option<i64>>,
    pub remind_room_id: Option<String>,
}

impl TodoPatch {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(ref text) = self.text {
            validate_text(text)?;
        }
        if let Some(Some(m)) = self.remind_before_minutes {
            if m <= 0 || m > crate::domain::reminders::MAX_LEAD_MINUTES {
                return Err(
                    "remind_before_minutes must be greater than zero and at most 43200 (30 days)",
                );
            }
        }
        Ok(())
    }
}

/// The instant a reminder should fire for `todo`, if it wants one at all.
/// A completed task never wants one.
pub fn wanted_trigger(todo: &TodoItem) -> Option<DateTime<Utc>> {
    match (todo.due_at, todo.remind_before_minutes, todo.completed) {
        (Some(due), Some(minutes), false) if minutes > 0 => Some(due - Duration::minutes(minutes)),
        _ => None,
    }
}

pub fn validate_text(text: &str) -> Result<(), &'static str> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("Todo text cannot be empty");
    }
    if text.len() > MAX_TEXT_LEN {
        return Err("Todo text exceeds 500 characters limit");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_todo_priority_conversion() {
        assert_eq!(
            TodoPriority::from_str_or_default("high"),
            TodoPriority::High
        );
        assert_eq!(TodoPriority::from_str_or_default("h"), TodoPriority::High);
        assert_eq!(TodoPriority::from_str_or_default("low"), TodoPriority::Low);
        assert_eq!(TodoPriority::from_str_or_default("l"), TodoPriority::Low);
        assert_eq!(
            TodoPriority::from_str_or_default("medium"),
            TodoPriority::Medium
        );
        assert_eq!(
            TodoPriority::from_str_or_default("anything_else"),
            TodoPriority::Medium
        );

        assert_eq!(TodoPriority::High.as_str(), "high");
        assert_eq!(TodoPriority::Medium.as_str(), "medium");
        assert_eq!(TodoPriority::Low.as_str(), "low");
    }

    #[test]
    fn test_wanted_trigger() {
        let now = Utc::now();
        let due = now + Duration::hours(5);

        let todo = TodoItem {
            id: 1,
            room_id: "!r:matrix.org".to_string(),
            owner_id: AccountId::new("acct_1"),
            sender_name: "alice".to_string(),
            text: "Buy groceries".to_string(),
            priority: TodoPriority::Medium,
            due_at: Some(due),
            remind_before_minutes: Some(60),
            completed: false,
            created_at: Some(now),
        };

        assert_eq!(wanted_trigger(&todo), Some(due - Duration::minutes(60)));

        // Completed -> None
        let mut completed = todo.clone();
        completed.completed = true;
        assert_eq!(wanted_trigger(&completed), None);

        // No due_at -> None
        let mut no_due = todo.clone();
        no_due.due_at = None;
        assert_eq!(wanted_trigger(&no_due), None);

        // 0 minutes -> None
        let mut zero_lead = todo.clone();
        zero_lead.remind_before_minutes = Some(0);
        assert_eq!(wanted_trigger(&zero_lead), None);
    }
}
