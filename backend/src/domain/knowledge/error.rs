use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KnowledgeError {
    #[error("note title cannot be empty")]
    EmptyTitle,
    #[error("note title too long (max {max} characters, got {actual})")]
    TitleTooLong { max: usize, actual: usize },
    #[error("note content cannot be empty")]
    EmptyContent,
    #[error("note content too long (max {max} characters, got {actual})")]
    ContentTooLong { max: usize, actual: usize },
    #[error("invalid note type: {0}")]
    InvalidNoteType(String),
    #[error("invalid content hash: {0}")]
    InvalidContentHash(String),
    #[error("note changed since it was loaded")]
    Conflict,
    #[error("invalid tag: {0}")]
    InvalidTag(String),
    #[error("note not found: {0}")]
    NotFound(i64),
}
