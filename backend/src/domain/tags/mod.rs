//! Shared tag value object used by notes, todos and any other taggable feature.
//!
//! A tag is trimmed, stripped of one leading run of `#`, lowercased and limited
//! to Unicode letters/digits plus `_` and `-`. The length limit counts
//! characters, so a 50-character CJK tag is accepted.

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_TAG_LEN: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TagError {
    #[error("tag cannot be empty")]
    Empty,
    #[error("tag cannot exceed {max} characters")]
    TooLong { max: usize },
    #[error("tag contains invalid characters: '{0}'")]
    InvalidChars(String),
}

/// A validated, normalized tag. Deserialization goes through [`Tag::parse`]
/// so a stored or wire value can never bypass the rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Tag(String);

impl Tag {
    pub fn parse(raw: &str) -> Result<Tag, TagError> {
        let norm = raw.trim().trim_start_matches('#').to_lowercase();
        if norm.is_empty() {
            return Err(TagError::Empty);
        }
        if norm.chars().count() > MAX_TAG_LEN {
            return Err(TagError::TooLong { max: MAX_TAG_LEN });
        }
        if !norm
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(TagError::InvalidChars(norm));
        }
        Ok(Tag(norm))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for Tag {
    type Error = TagError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Tag::parse(&value)
    }
}

impl From<Tag> for String {
    fn from(tag: Tag) -> String {
        tag.0
    }
}

/// Strict: the first invalid tag fails the whole list. Deduplicated and sorted.
pub fn parse_tags(raw: &[String]) -> Result<Vec<Tag>, TagError> {
    let mut tags = raw
        .iter()
        .map(|t| Tag::parse(t))
        .collect::<Result<Vec<_>, _>>()?;
    tags.sort();
    tags.dedup();
    Ok(tags)
}

/// Lenient: invalid tags are dropped. Deduplicated and sorted.
pub fn normalize_tags_lenient(raw: &[String]) -> Vec<Tag> {
    let mut tags: Vec<Tag> = raw.iter().filter_map(|t| Tag::parse(t).ok()).collect();
    tags.sort();
    tags.dedup();
    tags
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_strips_hash_and_lowercases() {
        assert_eq!(Tag::parse("  #Work ").unwrap().as_str(), "work");
    }

    #[test]
    fn parse_accepts_unicode() {
        assert_eq!(Tag::parse("#工作").unwrap().as_str(), "工作");
        assert_eq!(Tag::parse("Café").unwrap().as_str(), "café");
    }

    #[test]
    fn parse_rejects_empty() {
        for raw in ["", "#", "   "] {
            assert_eq!(Tag::parse(raw).unwrap_err(), TagError::Empty, "{raw:?}");
        }
    }

    #[test]
    fn parse_rejects_over_50_chars() {
        assert_eq!(
            Tag::parse(&"a".repeat(51)).unwrap_err(),
            TagError::TooLong { max: 50 }
        );
        assert!(Tag::parse(&"a".repeat(50)).is_ok());
        assert!(Tag::parse(&"工".repeat(50)).is_ok());
    }

    #[test]
    fn parse_rejects_spaces_and_punctuation() {
        for raw in ["a b", "a.b", "a#b"] {
            assert!(
                matches!(Tag::parse(raw).unwrap_err(), TagError::InvalidChars(_)),
                "{raw:?}"
            );
        }
    }

    #[test]
    fn parse_tags_dedupes_and_sorts() {
        let tags = parse_tags(&s(&["b", "#A", "a"])).unwrap();
        let out: Vec<&str> = tags.iter().map(Tag::as_str).collect();
        assert_eq!(out, ["a", "b"]);
        assert!(parse_tags(&s(&["ok", "bad tag"])).is_err());
    }

    #[test]
    fn lenient_drops_invalid() {
        let tags = normalize_tags_lenient(&s(&["ok", "bad tag", "OK"]));
        let out: Vec<&str> = tags.iter().map(Tag::as_str).collect();
        assert_eq!(out, ["ok"]);
    }

    #[test]
    fn every_previously_valid_ascii_tag_parses_identically() {
        for x in ["rust", "my-tag", "a_1", "ABC"] {
            assert_eq!(
                Tag::parse(x).unwrap().as_str(),
                x.trim_start_matches('#').to_ascii_lowercase()
            );
        }
    }

    #[test]
    fn deserialize_goes_through_parse() {
        let tag: Tag = serde_json::from_str("\"#Work\"").unwrap();
        assert_eq!(tag.as_str(), "work");
        assert!(serde_json::from_str::<Tag>("\"bad tag\"").is_err());
        assert_eq!(serde_json::to_string(&tag).unwrap(), "\"work\"");
    }
}
