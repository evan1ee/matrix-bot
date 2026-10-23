//! Checklist steps of a todo.

use super::TodoError;
use super::TodoItem;

/// Maximum number of steps on one todo.
pub const MAX_STEPS: usize = 50;
/// Maximum length of one step's text, in characters.
pub const MAX_STEP_TEXT_LEN: usize = 200;

/// One checklist item of a todo.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TodoStep {
    pub id: i64,
    pub todo_id: i64,
    pub position: i32,
    pub text: String,
    pub done: bool,
}

/// A todo together with its steps, for single-todo reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoWithSteps {
    pub todo: TodoItem,
    pub steps: Vec<TodoStep>,
}

impl TodoWithSteps {
    /// The step shown at 1-based `position`, as chat and AI callers address it.
    pub fn step_at(&self, position: usize) -> Option<&TodoStep> {
        position.checked_sub(1).and_then(|i| self.steps.get(i))
    }
}

/// Trims `text` and checks it is non-empty and at most `MAX_STEP_TEXT_LEN`
/// characters (counted as `char`s, so CJK text is not penalised).
pub fn validate_step_text(text: &str) -> Result<String, TodoError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(TodoError::StepTextEmpty);
    }
    if trimmed.chars().count() > MAX_STEP_TEXT_LEN {
        return Err(TodoError::StepTextTooLong);
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_text_is_trimmed_and_capped() {
        assert_eq!(validate_step_text("  buy milk ").unwrap(), "buy milk");
        assert!(validate_step_text(&"a".repeat(200)).is_ok());
        assert_eq!(
            validate_step_text(&"a".repeat(201)),
            Err(TodoError::StepTextTooLong)
        );
        assert_eq!(validate_step_text("  "), Err(TodoError::StepTextEmpty));
    }
}
