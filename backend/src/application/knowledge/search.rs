//! Embedding of knowledge items and the `SearchService` they feed. Querying
//! lives in `retrieval.rs`.

use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::search::{chunk_text, KnowledgeItemType, LEGACY_EMBEDDING_MODEL};
use crate::infrastructure::inference::embedding::EmbeddingProvider;
use crate::infrastructure::postgres::repositories;

/// How long a query waits for the embedder before search degrades to keywords.
const DEFAULT_EMBED_TIMEOUT: Duration = Duration::from_secs(3);

/// Service executing cross-domain hybrid semantic and keyword searches.
#[derive(Clone)]
pub struct SearchService {
    pub(super) pool: PgPool,
    pub(super) embedder: Arc<dyn EmbeddingProvider>,
    /// Only chunks produced by this model take part in vector search.
    pub(super) model: String,
    pub(super) embed_timeout: Duration,
}

impl SearchService {
    pub fn new(pool: PgPool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            pool,
            embedder,
            model: LEGACY_EMBEDDING_MODEL.to_string(),
            embed_timeout: DEFAULT_EMBED_TIMEOUT,
        }
    }

    pub fn with_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            embedder,
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

    /// Chunks, computes embeddings, and persists knowledge chunks for an item.
    pub async fn embed_item(
        &self,
        owner: &AccountId,
        item_type: KnowledgeItemType,
        item_id: i64,
    ) -> Result<(), AppError> {
        let content = repositories::knowledge::get_item_content_for_embedding(
            &self.pool, owner, item_type, item_id,
        )
        .await?;

        let Some(text) = content else {
            // Item removed or empty; purge any existing embeddings
            repositories::knowledge::delete_embeddings_for_item(
                &self.pool,
                owner,
                item_type.as_str(),
                item_id,
            )
            .await?;
            return Ok(());
        };

        let chunks = chunk_text(&text, 500, 100);
        if chunks.is_empty() {
            repositories::knowledge::delete_embeddings_for_item(
                &self.pool,
                owner,
                item_type.as_str(),
                item_id,
            )
            .await?;
            return Ok(());
        }

        let embeddings = self
            .embedder
            .embed(&chunks)
            .await
            .map_err(|e| AppError::External {
                service: "embeddings",
                source: anyhow::anyhow!("{e}"),
            })?;

        let mut tx = self.pool.begin().await?;
        repositories::knowledge::delete_embeddings_for_item(
            &mut *tx,
            owner,
            item_type.as_str(),
            item_id,
        )
        .await?;

        for (idx, (chunk, emb)) in chunks.iter().zip(embeddings.iter()).enumerate() {
            repositories::knowledge::upsert_embedding_chunk(
                &mut *tx,
                owner,
                item_type.as_str(),
                item_id,
                idx as i32,
                chunk,
                emb,
            )
            .await?;
        }
        tx.commit().await?;

        Ok(())
    }

    /// Scans unindexed items and enqueues low-priority background embedding jobs.
    pub async fn backfill_embeddings(&self, batch_size: i64) -> Result<usize, AppError> {
        let unindexed =
            repositories::knowledge::list_unindexed_items(&self.pool, batch_size).await?;
        let mut enqueued = 0;
        for item in unindexed {
            let job = NewJob::knowledge_embed_item(item.owner_id, item.item_type, item.item_id)
                .with_priority(-1);
            if repositories::jobs::enqueue(&self.pool, &job)
                .await?
                .is_some()
            {
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }
}

impl Default for SearchService {
    fn default() -> Self {
        panic!("SearchService must be initialized with a PgPool");
    }
}
