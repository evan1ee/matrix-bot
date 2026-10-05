//! PostgreSQL repository for Second Brain knowledge base & notes.
//!
//! Provides compile-checked `sqlx::query!` / `query_as!` access to `notes`
//! and `knowledge_embeddings` tables, including HNSW vector hybrid search.

pub mod notes;
pub mod search;

pub use notes::*;
pub use search::*;
