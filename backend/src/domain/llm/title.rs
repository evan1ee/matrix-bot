//! Conversation titles: the validated value and the heuristic that names a
//! thread from its first prompt.

use thiserror::Error;

pub const MAX_TITLE_CHARS: usize = 100;
const AUTO_TITLE_CHARS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TitleError {
    #[error("Title cannot be empty")]
    Empty,
    #[error("Title exceeds maximum length of {MAX_TITLE_CHARS} characters")]
    TooLong,
    #[error("Title must be a single line without control characters")]
    NotSingleLine,
}

/// A trimmed, single-line title of 1 to [`MAX_TITLE_CHARS`] characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTitle(String);

impl ConversationTitle {
    pub fn parse(raw: &str) -> Result<Self, TitleError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TitleError::Empty);
        }
        if trimmed.chars().any(char::is_control) {
            return Err(TitleError::NotSingleLine);
        }
        if trimmed.chars().count() > MAX_TITLE_CHARS {
            return Err(TitleError::TooLong);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Names a thread from its first prompt: the first non-blank line with
/// markdown markers and runs of whitespace removed, cut at a word boundary
/// within 50 characters. `None` when nothing printable is left.
pub fn title_from_prompt(prompt: &str) -> Option<ConversationTitle> {
    let line = prompt.lines().map(str::trim).find(|l| !l.is_empty())?;
    let cleaned: String = line
        .trim_start_matches(['#', '>', '*', '-', '`', ' '])
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.chars().count() <= AUTO_TITLE_CHARS {
        return ConversationTitle::parse(&cleaned).ok();
    }
    let cut: String = cleaned.chars().take(AUTO_TITLE_CHARS).collect();
    // Prefer the last whole word; a single long token is cut where it stands.
    let body = match cut.rfind(' ') {
        Some(at) if at > 0 => &cut[..at],
        _ => cut.as_str(),
    };
    ConversationTitle::parse(&format!("{}…", body.trim_end_matches([',', ';', ':', '.']))).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_bounds() {
        assert_eq!(ConversationTitle::parse("   "), Err(TitleError::Empty));
        assert_eq!(ConversationTitle::parse(""), Err(TitleError::Empty));
        assert_eq!(ConversationTitle::parse(" a ").unwrap().as_str(), "a");
        assert!(ConversationTitle::parse(&"x".repeat(100)).is_ok());
        assert_eq!(
            ConversationTitle::parse(&"x".repeat(101)),
            Err(TitleError::TooLong)
        );
        // Length counts characters, not bytes.
        assert!(ConversationTitle::parse(&"日".repeat(100)).is_ok());
    }

    #[test]
    fn title_rejects_newlines_and_control_characters() {
        assert_eq!(
            ConversationTitle::parse("two\nlines"),
            Err(TitleError::NotSingleLine)
        );
        assert_eq!(
            ConversationTitle::parse("bell\u{7}"),
            Err(TitleError::NotSingleLine)
        );
    }

    #[test]
    fn short_prompts_become_the_title_unchanged() {
        let t = title_from_prompt("What tasks do I have due today?").unwrap();
        assert_eq!(t.as_str(), "What tasks do I have due today?");
    }

    #[test]
    fn long_prompts_are_cut_at_a_word_boundary() {
        let t = title_from_prompt(
            "Spent $16.50 on lunch at Shake Shack, check off my workout habit, and remind me",
        )
        .unwrap();
        assert!(t.as_str().ends_with('…'));
        assert!(t.as_str().chars().count() <= AUTO_TITLE_CHARS + 1);
        assert!(!t.as_str().contains("remind"));
        assert!(t
            .as_str()
            .starts_with("Spent $16.50 on lunch at Shake Shack"));
    }

    #[test]
    fn one_long_word_is_cut_in_place() {
        let t = title_from_prompt(&"a".repeat(80)).unwrap();
        assert_eq!(t.as_str().chars().count(), AUTO_TITLE_CHARS + 1);
    }

    #[test]
    fn multibyte_prompts_are_never_split_mid_character() {
        let t = title_from_prompt(&"日本語".repeat(40)).unwrap();
        assert!(t.as_str().chars().count() <= AUTO_TITLE_CHARS + 1);
        let emoji = title_from_prompt(&"🙂 ".repeat(60)).unwrap();
        assert!(emoji.as_str().ends_with('…'));
    }

    #[test]
    fn markdown_markers_blank_lines_and_whitespace_are_dropped() {
        let t = title_from_prompt("\n\n##   Plan   my   week\nmore").unwrap();
        assert_eq!(t.as_str(), "Plan my week");
    }

    #[test]
    fn nothing_printable_gives_no_title() {
        assert!(title_from_prompt("   \n  ").is_none());
        assert!(title_from_prompt("###").is_none());
        assert!(title_from_prompt("").is_none());
    }
}
