//! Knowledge base (notes, bookmarks, journal, footprints) application services.
//!
//! Owns business use cases, transactional boundaries, and outbox event
//! publication for notes and cross-domain semantic hybrid search.

pub mod markdown;
mod notes;
mod retrieval;
mod search;

pub use notes::NoteService;
pub use retrieval::SearchQuery;
pub use search::SearchService;

use crate::application::AppError;
use crate::domain::knowledge::error::KnowledgeError;

impl From<KnowledgeError> for AppError {
    fn from(err: KnowledgeError) -> Self {
        match err {
            KnowledgeError::NotFound(id) => AppError::NotFound(format!("Note {id} not found")),
            KnowledgeError::Conflict => AppError::Conflict(KnowledgeError::Conflict.to_string()),
            other => AppError::Validation(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_stale_expected_hash_maps_to_conflict() {
        let err = AppError::from(KnowledgeError::Conflict);
        assert!(
            matches!(err, AppError::Conflict(ref m) if m == "note changed since it was loaded")
        );
    }

    #[test]
    fn validation_failures_map_to_validation() {
        let err = AppError::from(KnowledgeError::ContentTooLong { max: 1, actual: 2 });
        assert!(matches!(err, AppError::Validation(_)));
        let err = AppError::from(KnowledgeError::InvalidNoteType("x".into()));
        assert!(matches!(err, AppError::Validation(_)));
    }
}
