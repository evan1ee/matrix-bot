use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeItemType {
    Note,
    Bookmark,
    Journal,
    Footprint,
}

impl KnowledgeItemType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Bookmark => "bookmark",
            Self::Journal => "journal",
            Self::Footprint => "footprint",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "note" => Some(Self::Note),
            "bookmark" => Some(Self::Bookmark),
            "journal" => Some(Self::Journal),
            "footprint" => Some(Self::Footprint),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeChunk {
    pub item_type: KnowledgeItemType,
    pub item_id: i64,
    pub chunk_index: i32,
    pub content_snippet: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResultItem {
    pub item_type: KnowledgeItemType,
    pub item_id: i64,
    pub title: String,
    pub content_snippet: String,
    pub similarity_score: f32,
    pub source_ref: Option<String>,
}

/// Normalizes a cosine distance (typically 0.0 for identical, 2.0 for opposite)
/// into a similarity score bounded in [0.0, 1.0].
pub fn distance_to_similarity(distance: f32) -> f32 {
    let clamped = distance.clamp(0.0, 2.0);
    1.0 - (clamped / 2.0)
}

/// Chunks text into segments of at most `chunk_size` characters with `overlap` characters
/// of overlap between successive chunks. Character boundaries (Unicode scalar values)
/// are preserved. Empty or blank inputs yield an empty vector.
pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() || chunk_size == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= chunk_size {
        return vec![trimmed.to_string()];
    }

    let step = chunk_size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + chunk_size).min(chars.len());
        let snippet: String = chars[start..end].iter().collect();
        chunks.push(snippet);
        if end == chars.len() {
            break;
        }
        start += step;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_types_serialize_and_parse() {
        assert_eq!(KnowledgeItemType::Note.as_str(), "note");
        assert_eq!(KnowledgeItemType::Bookmark.as_str(), "bookmark");
        assert_eq!(KnowledgeItemType::Journal.as_str(), "journal");
        assert_eq!(KnowledgeItemType::Footprint.as_str(), "footprint");

        assert_eq!(
            KnowledgeItemType::from_str_opt("note"),
            Some(KnowledgeItemType::Note)
        );
        assert_eq!(
            KnowledgeItemType::from_str_opt("BOOKMARK"),
            Some(KnowledgeItemType::Bookmark)
        );
        assert_eq!(KnowledgeItemType::from_str_opt("invalid"), None);
    }

    #[test]
    fn converts_distance_to_similarity_cleanly() {
        assert!((distance_to_similarity(0.0) - 1.0).abs() < f32::EPSILON);
        assert!((distance_to_similarity(1.0) - 0.5).abs() < f32::EPSILON);
        assert!((distance_to_similarity(2.0) - 0.0).abs() < f32::EPSILON);
        assert!((distance_to_similarity(-0.5) - 1.0).abs() < f32::EPSILON);
        assert!((distance_to_similarity(3.0) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn chunk_text_splits_with_overlap_and_handles_utf8() {
        assert!(chunk_text("", 500, 100).is_empty());
        assert!(chunk_text("   ", 500, 100).is_empty());

        let short = "Hello world";
        assert_eq!(chunk_text(short, 500, 100), vec!["Hello world"]);

        // 10 chars: "0123456789", chunk_size 6, overlap 2 -> step 4
        // chunk 0: 0..6 ("012345")
        // chunk 1: 4..10 ("456789")
        let digits = "0123456789";
        let chunks = chunk_text(digits, 6, 2);
        assert_eq!(chunks, vec!["012345", "456789"]);

        // Unicode multibyte characters
        let unicode = "🦀 Rust 编程 🚀 Antigravity";
        let u_chunks = chunk_text(unicode, 8, 2);
        assert!(!u_chunks.is_empty());
        // verify concatenation preserves characters
        assert_eq!(u_chunks[0], "🦀 Rust 编");
    }
}
