//! Chat and message context domain models.

/// Represents a quoted message extracted from a chat reply or Markdown quote block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuotedMessage {
    /// Author / sender of the quoted message (if identifiable).
    pub author: Option<String>,
    /// Plain-text content of the quoted message.
    pub body: String,
    /// Event or message ID if known from in-reply-to relation.
    pub event_id: Option<String>,
}

impl QuotedMessage {
    /// Creates a new quoted message context.
    pub fn new(author: Option<String>, body: String, event_id: Option<String>) -> Self {
        Self {
            author,
            body,
            event_id,
        }
    }

    /// Formats the quoted context and user instruction into an augmented prompt for the AI assistant.
    pub fn to_context_prompt(&self, user_instruction: &str) -> String {
        let author_str = self.author.as_deref().unwrap_or("someone");
        let instruction = user_instruction.trim();
        let prompt_instruction = if instruction.is_empty() {
            "Please analyze, explain, or fact check this quoted message as appropriate."
        } else {
            instruction
        };

        format!(
            "[Context: Replying to quoted message from {}]:\n\"\"\"\n{}\n\"\"\"\n\nUser request: {}",
            author_str,
            self.body.trim(),
            prompt_instruction
        )
    }
}

/// A tracked chat message target linked to a domain entity (e.g. todo, reminder).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BotMessageTarget {
    pub event_id: String,
    pub room_id: String,
    pub entity_type: String,
    pub entity_id: i64,
    pub owner_id: String,
    pub title: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_context_prompt_formatting() {
        let q = QuotedMessage::new(
            Some("@alice:matrix.org".to_string()),
            "Hello world".to_string(),
            Some("$ev1".to_string()),
        );
        let prompt = q.to_context_prompt("Summarize this");
        assert!(prompt.contains("@alice:matrix.org"));
        assert!(prompt.contains("Hello world"));
        assert!(prompt.contains("User request: Summarize this"));

        let empty_prompt = q.to_context_prompt("");
        assert!(empty_prompt.contains("Please analyze, explain, or fact check"));
    }
}
