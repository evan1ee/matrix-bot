//! The `SearchService` and its configuration. Querying lives in
//! `retrieval.rs`; embedding items and the backfill live in `indexing.rs`.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::domain::knowledge::search::DEFAULT_EMBEDDING_MODEL;
use crate::infrastructure::inference::embedding::EmbeddingProvider;

/// How long a query waits for the embedder before search degrades to keywords.
const DEFAULT_EMBED_TIMEOUT: Duration = Duration::from_secs(3);

/// Service executing cross-domain hybrid semantic and keyword searches and
/// keeping the embeddings behind them current.
#[derive(Clone)]
pub struct SearchService {
    pub(super) pool: PgPool,
    pub(super) embedder: Arc<dyn EmbeddingProvider>,
    /// The model new chunks are tagged with and the only one whose chunks take
    /// part in vector search. Changing it re-embeds everything via the backfill.
    pub(super) model: String,
    pub(super) embed_timeout: Duration,
}

impl SearchService {
    pub fn new(pool: PgPool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            pool,
            embedder,
            model: DEFAULT_EMBEDDING_MODEL.to_string(),
            embed_timeout: DEFAULT_EMBED_TIMEOUT,
        }
    }

    pub fn with_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            embedder,
            ..self.clone()
        }
    }

    /// Sets the configured embedding model.
    pub fn with_model(&self, model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..self.clone()
        }
    }

    /// Overrides the embed timeout; tests use it to exercise the slow path.
    #[cfg(test)]
    pub(super) fn with_embed_timeout(&self, embed_timeout: Duration) -> Self {
        Self {
            embed_timeout,
            ..self.clone()
        }
    }
}

impl Default for SearchService {
    fn default() -> Self {
        panic!("SearchService must be initialized with a PgPool");
    }
}
