//! Todo domain entities, value objects and invariants.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::domain::accounts::AccountId;
use crate::domain::reminders::RecurrencePattern;
use crate::domain::tags::Tag;

pub mod steps;

/// Maximum number of tags on one todo.
pub const MAX_TAGS: usize = 10;

/// Why a todo (or one of its steps) was refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TodoError {
    #[error("A todo can have at most 10 tags")]
    TooManyTags,
    #[error("A recurring todo needs a due date")]
    RecurrenceNeedsDue,
    #[error("A todo can have at most 50 steps")]
    TooManySteps,
    #[error("Step text cannot be empty")]
    StepTextEmpty,
    #[error("Step text exceeds 200 characters limit")]
    StepTextTooLong,
    #[error("Step not found")]
    StepNotFound,
    #[error("Step order must list every step of the todo exactly once")]
    StepOrderMismatch,
    #[error("{0}")]
    Text(&'static str),
}

impl From<&'static str> for TodoError {
    fn from(s: &'static str) -> Self {
        TodoError::Text(s)
    }
}

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
    #[serde(default)]
    pub tags: Vec<Tag>,
    #[serde(default)]
    pub recurrence: RecurrencePattern,
    /// The due date the recurring series started from; successors inherit it.
    #[serde(default)]
    pub recurrence_anchor: Option<DateTime<Utc>>,
    #[serde(default)]
    pub next_todo_id: Option<i64>,
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
    pub tags: Vec<Tag>,
    pub recurrence: RecurrencePattern,
    /// `None` from interfaces; the service derives it from `due_at`.
    pub recurrence_anchor: Option<DateTime<Utc>>,
}

impl NewTodo {
    pub fn validate(&self) -> Result<(), TodoError> {
        validate_text(&self.text)?;
        if self.tags.len() > MAX_TAGS {
            return Err(TodoError::TooManyTags);
        }
        if self.recurrence != RecurrencePattern::None && self.due_at.is_none() {
            return Err(TodoError::RecurrenceNeedsDue);
        }
        if self.remind_before_minutes.is_some() && self.due_at.is_none() {
            return Err("remind_before_minutes requires a due_at to count back from".into());
        }
        if let Some(m) = self.remind_before_minutes {
            if m <= 0 || m > crate::domain::reminders::MAX_LEAD_MINUTES {
                return Err(
                    "remind_before_minutes must be greater than zero and at most 43200 (30 days)"
                        .into(),
                );
            }
            let has_room = match self.remind_room_id.as_deref() {
                Some(r) => !r.trim().is_empty() && r.trim() != crate::domain::rooms::WEB_ROOM,
                None => {
                    self.room_id != crate::domain::rooms::WEB_ROOM
                        && !self.room_id.trim().is_empty()
                }
            };
            if !has_room {
                return Err(
                    "remind_room_id is required for a reminder on a task that has no Matrix room"
                        .into(),
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
    /// Whole-set replace.
    pub tags: Option<Vec<Tag>>,
    pub recurrence: Option<RecurrencePattern>,
    /// Set by the service, never by an interface: the series anchor to write.
    pub recurrence_anchor: Option<Option<DateTime<Utc>>>,
}

impl TodoPatch {
    pub fn validate(&self) -> Result<(), TodoError> {
        if let Some(ref text) = self.text {
            validate_text(text)?;
        }
        if self.tags.as_ref().is_some_and(|t| t.len() > MAX_TAGS) {
            return Err(TodoError::TooManyTags);
        }
        if let Some(Some(m)) = self.remind_before_minutes {
            if m <= 0 || m > crate::domain::reminders::MAX_LEAD_MINUTES {
                return Err(
                    "remind_before_minutes must be greater than zero and at most 43200 (30 days)"
                        .into(),
                );
            }
        }
        Ok(())
    }
}

/// The invariants of a todo after a patch has been applied to it: a recurring
/// todo has a due date and at most `MAX_TAGS` tags.
pub fn validate_merged(todo: &TodoItem) -> Result<(), TodoError> {
    if todo.tags.len() > MAX_TAGS {
        return Err(TodoError::TooManyTags);
    }
    if todo.recurrence != RecurrencePattern::None && todo.due_at.is_none() {
        return Err(TodoError::RecurrenceNeedsDue);
    }
    Ok(())
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
            owner_id: AccountId::from_seed("acct_1"),
            sender_name: "alice".to_string(),
            text: "Buy groceries".to_string(),
            priority: TodoPriority::Medium,
            due_at: Some(due),
            remind_before_minutes: Some(60),
            completed: false,
            created_at: Some(now),
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
            next_todo_id: None,
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

    fn new_todo() -> NewTodo {
        NewTodo {
            room_id: "web".to_string(),
            sender_name: "alice".to_string(),
            text: "water plants".to_string(),
            priority: TodoPriority::Medium,
            due_at: None,
            remind_before_minutes: None,
            remind_room_id: None,
            tags: vec![],
            recurrence: RecurrencePattern::None,
            recurrence_anchor: None,
        }
    }

    fn tags(n: usize) -> Vec<Tag> {
        (0..n)
            .map(|i| Tag::parse(&format!("t{i}")).unwrap())
            .collect()
    }

    #[test]
    fn new_todo_rejects_recurrence_without_due() {
        let input = NewTodo {
            recurrence: RecurrencePattern::Monthly,
            due_at: None,
            ..new_todo()
        };
        assert_eq!(input.validate(), Err(TodoError::RecurrenceNeedsDue));
    }

    #[test]
    fn new_todo_rejects_eleven_tags() {
        let ten = NewTodo {
            tags: tags(10),
            ..new_todo()
        };
        assert_eq!(ten.validate(), Ok(()));
        let eleven = NewTodo {
            tags: tags(11),
            ..new_todo()
        };
        assert_eq!(eleven.validate(), Err(TodoError::TooManyTags));
    }

    #[test]
    fn validate_merged_rejects_recurring_without_due() {
        let mut todo: TodoItem = serde_json::from_value(serde_json::json!({
            "id": 1, "room_id": "web", "owner_id": AccountId::from_seed("m"),
            "sender_name": "a", "text": "t", "priority": "low", "due_at": null,
            "remind_before_minutes": null, "completed": false, "created_at": null
        }))
        .unwrap();
        assert_eq!(validate_merged(&todo), Ok(()));
        todo.recurrence = RecurrencePattern::Daily;
        assert_eq!(validate_merged(&todo), Err(TodoError::RecurrenceNeedsDue));
        todo.due_at = Some(Utc::now());
        assert_eq!(validate_merged(&todo), Ok(()));
        todo.tags = tags(11);
        assert_eq!(validate_merged(&todo), Err(TodoError::TooManyTags));
    }

    #[test]
    fn todo_item_deserializes_without_new_fields() {
        let todo: TodoItem = serde_json::from_value(serde_json::json!({
            "id": 1, "room_id": "web", "owner_id": AccountId::from_seed("m"),
            "sender_name": "a", "text": "t", "priority": "low", "due_at": null,
            "remind_before_minutes": null, "completed": false, "created_at": null
        }))
        .unwrap();
        assert!(todo.tags.is_empty());
        assert_eq!(todo.recurrence, RecurrencePattern::None);
        assert_eq!(todo.recurrence_anchor, None);
        assert_eq!(todo.next_todo_id, None);
    }
}
