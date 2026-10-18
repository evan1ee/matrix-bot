//! Why an LLM/AI turn did not produce an answer.

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
    /// The caller stopped the turn (the web client closed its stream). The
    /// stored conversation already ends with a stopped marker.
    Cancelled,
}

impl std::fmt::Display for AiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AiError::Validation(msg) => write!(f, "{}", msg),
            AiError::Provider(msg) => write!(f, "{}", msg),
            AiError::Timeout => write!(f, "The AI provider did not respond in time"),
            AiError::Db(msg) => write!(f, "{}", msg),
            AiError::Cancelled => write!(f, "The turn was stopped"),
        }
    }
}

pub type LlmError = AiError;

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
        assert_eq!(AiError::Cancelled.to_string(), "The turn was stopped");
    }
}
