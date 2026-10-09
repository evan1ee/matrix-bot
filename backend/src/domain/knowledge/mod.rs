pub mod error;
pub mod note;
pub mod search;

pub use error::KnowledgeError;
pub use note::{
    normalize_tag, normalize_tags, validate_content, validate_title, ContentHash, ExpectedHash,
    NewNote, Note, NoteDocument, NoteId, NotePatch, NoteType, MAX_CONTENT_CHARS, MAX_TITLE_LEN,
};
pub use search::{
    distance_to_similarity, rrf, ItemKey, KnowledgeChunk, KnowledgeItemType, SearchMode,
    SearchResultItem,
};
