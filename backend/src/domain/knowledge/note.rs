use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::KnowledgeError;
use crate::domain::accounts::AccountId;

pub const MAX_TITLE_LEN: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NoteId(pub i64);

impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoteFormat {
    #[default]
    Markdown,
    Plain,
    Quote,
}

impl std::str::FromStr for NoteFormat {
    type Err = KnowledgeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "markdown" | "md" => Ok(Self::Markdown),
            "plain" | "text" | "txt" => Ok(Self::Plain),
            "quote" => Ok(Self::Quote),
            _ => Ok(Self::Markdown),
        }
    }
}

impl NoteFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::Plain => "plain",
            Self::Quote => "quote",
        }
    }
}

impl std::fmt::Display for NoteFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub owner_id: AccountId,
    pub title: String,
    pub content: String,
    pub format: NoteFormat,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub is_archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn validate_title(title: &str) -> Result<String, KnowledgeError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(KnowledgeError::EmptyTitle);
    }
    if trimmed.chars().count() > MAX_TITLE_LEN {
        return Err(KnowledgeError::TitleTooLong {
            max: MAX_TITLE_LEN,
            actual: trimmed.chars().count(),
        });
    }
    Ok(trimmed.to_string())
}

pub fn validate_content(content: &str) -> Result<String, KnowledgeError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(KnowledgeError::EmptyContent);
    }
    Ok(content.to_string())
}

pub fn normalize_tag(tag: &str) -> Result<String, KnowledgeError> {
    let raw = tag.trim().trim_start_matches('#').to_ascii_lowercase();
    if raw.is_empty() {
        return Err(KnowledgeError::InvalidTag(
            "tag cannot be empty".to_string(),
        ));
    }
    if raw.len() > 50 {
        return Err(KnowledgeError::InvalidTag(
            "tag cannot exceed 50 characters".to_string(),
        ));
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(KnowledgeError::InvalidTag(format!(
            "tag contains invalid characters: '{}'",
            raw
        )));
    }
    Ok(raw)
}

pub fn normalize_tags(tags: &[String]) -> Vec<String> {
    let mut normalized = Vec::new();
    for t in tags {
        if let Ok(clean) = normalize_tag(t) {
            if !normalized.contains(&clean) {
                normalized.push(clean);
            }
        }
    }
    normalized.sort();
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_titles_properly() {
        assert_eq!(validate_title(" Hello World ").unwrap(), "Hello World");
        assert_eq!(
            validate_title("   ").unwrap_err(),
            KnowledgeError::EmptyTitle
        );
        let too_long = "a".repeat(201);
        assert!(matches!(
            validate_title(&too_long).unwrap_err(),
            KnowledgeError::TitleTooLong {
                max: 200,
                actual: 201
            }
        ));
    }

    #[test]
    fn validates_content_properly() {
        assert_eq!(validate_content("note body").unwrap(), "note body");
        assert_eq!(
            validate_content("   ").unwrap_err(),
            KnowledgeError::EmptyContent
        );
    }

    #[test]
    fn normalizes_tags_cleanly() {
        assert_eq!(normalize_tag("#Rust").unwrap(), "rust");
        assert_eq!(normalize_tag("deep_learning").unwrap(), "deep_learning");
        assert_eq!(normalize_tag("ai-tools").unwrap(), "ai-tools");
        assert!(normalize_tag("invalid tag!").is_err());
        assert!(normalize_tag("").is_err());

        let raw = vec![
            "#rust".to_string(),
            "RUST".to_string(),
            "#AI".to_string(),
            "invalid space".to_string(),
        ];
        assert_eq!(
            normalize_tags(&raw),
            vec!["ai".to_string(), "rust".to_string()]
        );
    }

    #[test]
    fn format_parsing_works() {
        assert_eq!(
            "markdown".parse::<NoteFormat>().unwrap(),
            NoteFormat::Markdown
        );
        assert_eq!("quote".parse::<NoteFormat>().unwrap(), NoteFormat::Quote);
        assert_eq!("plain".parse::<NoteFormat>().unwrap(), NoteFormat::Plain);
        assert_eq!(
            "unknown".parse::<NoteFormat>().unwrap(),
            NoteFormat::Markdown
        );
    }
}
