pub mod error;
pub mod note;
pub mod search;

pub use error::KnowledgeError;
pub use note::{
    normalize_tag, normalize_tags, validate_content, validate_title, ContentHash, ExpectedHash,
    NewNote, Note, NoteDocument, NoteId, NotePatch, NoteType, MAX_CONTENT_CHARS, MAX_TITLE_LEN,
};
pub use search::{
    contains_cjk, distance_to_similarity, embedding_prefixes, rrf, EmbeddingPrefixes, IndexOutcome,
    ItemKey, KnowledgeChunk, KnowledgeItemType, SearchMode, SearchOutcome, SearchResultItem,
    DEFAULT_EMBEDDING_MODEL,
};
