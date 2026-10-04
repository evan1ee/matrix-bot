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
}
