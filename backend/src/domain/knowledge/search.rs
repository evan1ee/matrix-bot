use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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

/// The embedding model used when none is configured. Mixed English and Chinese
/// notes need a multilingual model; the old English-only default produced
/// near-meaningless vectors for Chinese.
pub const DEFAULT_EMBEDDING_MODEL: &str = "multilingual-e5-small";

/// Text prepended before embedding, which asymmetric retrieval models require:
/// a query and a stored passage are embedded differently. Not part of any
/// stored snippet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbeddingPrefixes {
    pub query: &'static str,
    pub passage: &'static str,
}

/// The prefixes `model` expects. E5 models are trained with `query: ` and
/// `passage: `; any other model gets none.
pub fn embedding_prefixes(model: &str) -> EmbeddingPrefixes {
    if model.to_ascii_lowercase().contains("e5") {
        EmbeddingPrefixes {
            query: "query: ",
            passage: "passage: ",
        }
    } else {
        EmbeddingPrefixes {
            query: "",
            passage: "",
        }
    }
}

/// What `embed_item` did, so the job handler can tell a no-op from work and
/// from a write that lost a race.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexOutcome {
    /// Chunks were (re)built and stored.
    Indexed,
    /// The stored chunks already match the current content and model.
    Skipped,
    /// The item is gone or has no text; its chunks were removed.
    Removed,
    /// The item changed while it was being embedded, so nothing was written
    /// and the work must be redone.
    Stale,
}

/// What a search returned and how it was produced.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchOutcome {
    pub items: Vec<SearchResultItem>,
    pub mode: SearchMode,
}

/// Whether `text` contains Han, kana or Hangul. The `simple` text-search
/// configuration cannot split these scripts into words, so such queries must
/// also be matched by trigram similarity.
pub fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c,
            '\u{3040}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7AF}'
            | '\u{F900}'..='\u{FAFF}')
    })
}

/// Identifies one searchable item across the four knowledge item types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemKey {
    pub item_type: KnowledgeItemType,
    pub item_id: i64,
}

impl ItemKey {
    pub fn new(item_type: KnowledgeItemType, item_id: i64) -> Self {
        Self { item_type, item_id }
    }
}

/// Whether the vector list contributed to a search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    Hybrid,
    /// The embedder failed or timed out; only keyword results are returned.
    Lexical,
}

impl SearchMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Lexical => "lexical",
        }
    }
}

/// Reciprocal rank fusion of a lexical and a vector ranking (best first).
///
/// Each item scores `sum(1 / (k + rank))` over the lists that contain it, with
/// 1-based ranks, and appears once however many chunks matched. A repeated key
/// within one list counts only at its first position. Results are ordered by
/// fused score (descending), then by earlier lexical rank, then by lower item
/// id, so equal scores never reorder between runs.
pub fn rrf(lexical: &[ItemKey], vector: &[ItemKey], k: u32) -> Vec<(ItemKey, f64)> {
    let mut scores: HashMap<ItemKey, (f64, usize)> = HashMap::new();
    let mut accumulate = |list: &[ItemKey], is_lexical: bool| {
        let mut seen: HashSet<ItemKey> = HashSet::new();
        for key in list {
            if !seen.insert(*key) {
                continue;
            }
            let rank = seen.len();
            let entry = scores.entry(*key).or_insert((0.0, usize::MAX));
            entry.0 += 1.0 / (f64::from(k) + rank as f64);
            if is_lexical {
                entry.1 = rank;
            }
        }
    };
    accumulate(lexical, true);
    accumulate(vector, false);

    let mut fused: Vec<(ItemKey, f64, usize)> = scores
        .into_iter()
        .map(|(key, (score, lexical_rank))| (key, score, lexical_rank))
        .collect();
    fused.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then(a.2.cmp(&b.2))
            .then(a.0.item_id.cmp(&b.0.item_id))
            .then(a.0.item_type.cmp(&b.0.item_type))
    });
    fused
        .into_iter()
        .map(|(key, score, _)| (key, score))
        .collect()
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

    fn key(id: i64) -> ItemKey {
        ItemKey::new(KnowledgeItemType::Note, id)
    }

    #[test]
    fn rrf_rewards_items_present_in_both_lists() {
        let fused = rrf(&[key(1), key(2)], &[key(2), key(3)], 60);
        let order: Vec<i64> = fused.iter().map(|(k, _)| k.item_id).collect();
        assert_eq!(order, vec![2, 1, 3]);
        let expected = 1.0 / 62.0 + 1.0 / 61.0;
        assert!((fused[0].1 - expected).abs() < 1e-12);
    }

    #[test]
    fn rrf_returns_each_item_once_even_if_a_list_repeats_it() {
        let fused = rrf(&[key(1), key(1), key(2)], &[key(1)], 60);
        assert_eq!(fused.len(), 2);
        // The duplicate does not add a second contribution or shift later ranks.
        let first = 1.0 / 61.0 + 1.0 / 61.0;
        assert!((fused[0].1 - first).abs() < 1e-12);
        assert_eq!(fused[1].0, key(2));
    }

    #[test]
    fn rrf_breaks_equal_scores_by_earlier_lexical_rank() {
        // 7 is only lexical rank 1 and 3 only vector rank 1: equal scores.
        let fused = rrf(&[key(7)], &[key(3)], 60);
        assert_eq!(fused[0].0, key(7));
        assert_eq!(fused[1].0, key(3));
        assert!((fused[0].1 - fused[1].1).abs() < f64::EPSILON);

        // 9 and 2 swap ranks across the lists, so they tie exactly; the earlier
        // lexical rank wins even though its item id is higher.
        let fused = rrf(&[key(9), key(2)], &[key(2), key(9)], 60);
        assert_eq!(fused[0].0, key(9));
        assert_eq!(fused[1].0, key(2));
    }

    #[test]
    fn rrf_ranks_by_score_before_any_tie_break() {
        // 5 is lexical 3 and vector 1; 8 is lexical 2 and vector 2.
        let fused = rrf(&[key(1), key(8), key(5)], &[key(5), key(8)], 60);
        assert_eq!(fused[0].0, key(5));
        assert_eq!(fused[1].0, key(8));
    }

    #[test]
    fn rrf_of_two_empty_lists_is_empty() {
        assert!(rrf(&[], &[], 60).is_empty());
    }

    #[test]
    fn e5_models_get_query_and_passage_prefixes_and_others_none() {
        let e5 = embedding_prefixes(DEFAULT_EMBEDDING_MODEL);
        assert_eq!((e5.query, e5.passage), ("query: ", "passage: "));
        assert_eq!(embedding_prefixes("Multilingual-E5-Large"), e5);
        let bge = embedding_prefixes("bge-small-en-v1.5");
        assert_eq!((bge.query, bge.passage), ("", ""));
    }

    #[test]
    fn detects_cjk_scripts() {
        assert!(contains_cjk("猫"));
        assert!(contains_cjk("rust 与 tokio"));
        assert!(contains_cjk("ひらがな"));
        assert!(contains_cjk("한국어"));
        assert!(!contains_cjk("plain ascii and accents é"));
        assert!(!contains_cjk(""));
    }

    #[test]
    fn search_mode_has_stable_wire_names() {
        assert_eq!(SearchMode::Hybrid.as_str(), "hybrid");
        assert_eq!(SearchMode::Lexical.as_str(), "lexical");
    }
}
