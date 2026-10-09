use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::error::KnowledgeError;
use crate::domain::accounts::AccountId;

pub const MAX_TITLE_LEN: usize = 200;
/// Largest accepted note body in characters. Without a cap one request could
/// store an unbounded body and an unbounded embedding fan-out.
pub const MAX_CONTENT_CHARS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NoteId(pub i64);

impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Hex SHA-256 of a note's Markdown. Compared to detect a changed body without
/// loading it (optimistic concurrency, embedding skip).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ContentHash(String);

impl ContentHash {
    /// Accepts exactly 64 lowercase hexadecimal characters.
    pub fn from_hex(hex: &str) -> Result<Self, KnowledgeError> {
        let valid = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        if valid {
            Ok(Self(hex.to_string()))
        } else {
            Err(KnowledgeError::InvalidContentHash(hex.to_string()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ContentHash {
    type Error = KnowledgeError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::from_hex(&value)
    }
}

impl From<ContentHash> for String {
    fn from(hash: ContentHash) -> Self {
        hash.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a note is, independent of how it was captured. Unknown values are an
/// error: silently defaulting would hide a typo in a client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NoteType {
    #[default]
    Note,
    Idea,
    Fact,
    Observation,
    Decision,
}

impl NoteType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Idea => "idea",
            Self::Fact => "fact",
            Self::Observation => "observation",
            Self::Decision => "decision",
        }
    }
}

impl std::str::FromStr for NoteType {
    type Err = KnowledgeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "note" => Ok(Self::Note),
            "idea" => Ok(Self::Idea),
            "fact" => Ok(Self::Fact),
            "observation" => Ok(Self::Observation),
            "decision" => Ok(Self::Decision),
            _ => Err(KnowledgeError::InvalidNoteType(s.trim().to_string())),
        }
    }
}

impl std::fmt::Display for NoteType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub owner_id: AccountId,
    pub title: String,
    pub note_type: NoteType,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    /// First characters of the plain text, for lists. Derived on write.
    pub excerpt: String,
    pub content_hash: ContentHash,
    pub is_archived: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When the embeddings were last built from `content_hash`.
    pub indexed_at: Option<DateTime<Utc>>,
}

/// Input for creating a note. Validation and tag normalisation happen in the
/// service, not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNote {
    pub title: String,
    pub markdown: String,
    pub note_type: NoteType,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    /// Created already archived (an import of an archived note).
    pub is_archived: bool,
}

impl NewNote {
    /// A web note with no tags.
    pub fn new(title: impl Into<String>, markdown: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            markdown: markdown.into(),
            note_type: NoteType::default(),
            source_channel: "web".to_string(),
            source_ref: None,
            tags: Vec::new(),
            is_archived: false,
        }
    }

    pub fn note_type(mut self, note_type: NoteType) -> Self {
        self.note_type = note_type;
        self
    }

    pub fn source(mut self, channel: impl Into<String>, source_ref: Option<String>) -> Self {
        self.source_channel = channel.into();
        self.source_ref = source_ref;
        self
    }

    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn archived(mut self, is_archived: bool) -> Self {
        self.is_archived = is_archived;
        self
    }
}

/// A note together with its Markdown body. Lists and events carry `Note`
/// alone; only a read of one note loads the document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteDocument {
    pub note: Note,
    pub markdown: String,
}

/// The caller's expectation of the stored body, checked under the row lock.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ExpectedHash {
    /// Last writer wins (chat, MCP, LLM appends).
    #[default]
    Any,
    /// The web editor: fail with `Conflict` if the body changed since it loaded.
    Exactly(ContentHash),
}

/// A partial update: `None` leaves the field as stored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NotePatch {
    pub title: Option<String>,
    pub markdown: Option<String>,
    pub tags: Option<Vec<String>>,
    pub note_type: Option<NoteType>,
    pub is_archived: Option<bool>,
    pub expected: ExpectedHash,
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
    let chars = content.chars().count();
    if chars > MAX_CONTENT_CHARS {
        return Err(KnowledgeError::ContentTooLong {
            max: MAX_CONTENT_CHARS,
            actual: chars,
        });
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
    fn content_hash_accepts_only_lowercase_sha256_hex() {
        let ok = "a".repeat(64);
        assert_eq!(ContentHash::from_hex(&ok).unwrap().as_str(), ok);
        assert!(ContentHash::from_hex(&"a".repeat(63)).is_err());
        assert!(ContentHash::from_hex(&"A".repeat(64)).is_err());
        assert!(ContentHash::from_hex(&"g".repeat(64)).is_err());
        assert!(ContentHash::from_hex("").is_err());
    }

    #[test]
    fn note_type_parses_known_values_and_rejects_unknown() {
        for (raw, ty) in [
            ("note", NoteType::Note),
            ("idea", NoteType::Idea),
            ("fact", NoteType::Fact),
            ("observation", NoteType::Observation),
            ("decision", NoteType::Decision),
        ] {
            assert_eq!(raw.parse::<NoteType>().unwrap(), ty);
            assert_eq!(ty.as_str(), raw);
        }
        assert_eq!(
            " Decision ".parse::<NoteType>().unwrap(),
            NoteType::Decision
        );
        assert_eq!(
            "journal".parse::<NoteType>().unwrap_err(),
            KnowledgeError::InvalidNoteType("journal".to_string())
        );
        assert_eq!(NoteType::default(), NoteType::Note);
    }

    #[test]
    fn content_is_capped_at_the_maximum_length() {
        let at_cap = "a".repeat(MAX_CONTENT_CHARS);
        assert!(validate_content(&at_cap).is_ok());
        let over = "a".repeat(MAX_CONTENT_CHARS + 1);
        assert_eq!(
            validate_content(&over).unwrap_err(),
            KnowledgeError::ContentTooLong {
                max: MAX_CONTENT_CHARS,
                actual: MAX_CONTENT_CHARS + 1
            }
        );
        // The cap counts characters, not bytes.
        let cjk = "猫".repeat(MAX_CONTENT_CHARS);
        assert!(validate_content(&cjk).is_ok());
    }
}
