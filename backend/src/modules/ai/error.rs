//! Why an AI turn did not produce an answer.
//!
//! `run_ai_turn` and `run_ai_turn_with_stage` used to return a bare `String`,
//! so `api/ai.rs::chat` had no way to tell a prompt it should not have
//! accepted apart from a provider that was down — both answered 400 Bad
//! Request, with the raw internal message in the body. This type lets the
//! REST layer answer each failure with the status it actually deserves.
//!
//! Kept in its own file rather than folded into `handler.rs`: that file is
//! already at its line-count ceiling.

/// Why an AI turn did not produce an answer.
#[derive(Debug)]
pub enum AiError {
    /// The request itself is wrong: an empty or over-long prompt, or a
    /// conversation id that does not exist for this owner.
    Validation(String),
    /// The configured provider refused, failed, or is not configured.
    Provider(String),
    /// The provider did not answer in time.
    Timeout,
    Db(String),
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::Validation(msg) => write!(f, "{}", msg),
            AiError::Provider(msg) => write!(f, "{}", msg),
            AiError::Timeout => write!(f, "The AI provider did not respond in time"),
            AiError::Db(msg) => write!(f, "{}", msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_reuses_the_inner_message() {
        assert_eq!(
            AiError::Validation("Prompt cannot be empty".to_string()).to_string(),
            "Prompt cannot be empty"
        );
        assert_eq!(
            AiError::Provider("connection refused".to_string()).to_string(),
            "connection refused"
        );
        assert_eq!(
            AiError::Db("db exploded".to_string()).to_string(),
            "db exploded"
        );
        assert_eq!(
            AiError::Timeout.to_string(),
            "The AI provider did not respond in time"
        );
    }
}
