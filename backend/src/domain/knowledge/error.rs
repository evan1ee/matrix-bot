use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KnowledgeError {
    #[error("note title cannot be empty")]
    EmptyTitle,
    #[error("note title too long (max {max} characters, got {actual})")]
    TitleTooLong { max: usize, actual: usize },
    #[error("note content cannot be empty")]
    EmptyContent,
    #[error("invalid tag: {0}")]
    InvalidTag(String),
    #[error("note not found: {0}")]
    NotFound(i64),
}
