//! PostgreSQL repository for the notes knowledge base.
//!
//! Provides compile-checked `sqlx::query!` / `query_as!` access to `notes`
//! and `knowledge_embeddings` tables, including HNSW vector hybrid search.

pub mod indexing;
pub mod notes;
pub mod retrieval;
pub mod search;

pub use indexing::*;
pub use notes::*;
pub use retrieval::*;
pub use search::*;
