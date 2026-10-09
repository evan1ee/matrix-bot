//! Cross-domain hybrid search and the embedding of knowledge items.

use sqlx::PgPool;
use std::sync::Arc;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::search::{chunk_text, KnowledgeItemType, SearchResultItem};
use crate::infrastructure::inference::embedding::EmbeddingProvider;
use crate::infrastructure::postgres::repositories;

/// Service executing cross-domain hybrid semantic and BM25 searches.
#[derive(Clone)]
pub struct SearchService {
    pool: PgPool,
    embedder: Arc<dyn EmbeddingProvider>,
}

impl SearchService {
    pub fn new(pool: PgPool, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self { pool, embedder }
    }

    pub fn with_embedder(&self, embedder: Arc<dyn EmbeddingProvider>) -> Self {
        Self {
            pool: self.pool.clone(),
            embedder,
        }
    }

    /// Searches across indexed personal knowledge items for `owner`.
    pub async fn search(
        &self,
        owner: &AccountId,
        query: &str,
        filter_types: Option<&[String]>,
        limit: i64,
    ) -> Result<Vec<SearchResultItem>, AppError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let limit = limit.clamp(1, 50);

        let vectors = self
            .embedder
            .embed(&[trimmed.to_string()])
            .await
            .map_err(|e| AppError::External {
                service: "embeddings",
                source: anyhow::anyhow!("{e}"),
            })?;

        let query_vector = vectors
            .into_iter()
            .next()
            .ok_or_else(|| AppError::External {
                service: "embeddings",
                source: anyhow::anyhow!("no embedding vector returned"),
            })?;

        let results = repositories::knowledge::search_hybrid(
            &self.pool,
            owner,
            &query_vector,
            trimmed,
            filter_types,
            limit,
        )
        .await?;

        Ok(results)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::knowledge::NoteService;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::domain::knowledge::note::NewNote;
    use crate::infrastructure::inference::embedding::MockEmbeddingProvider;

    #[tokio::test]
    async fn test_search_service_with_mock_embedder() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let note_svc = NoteService::new(pool.clone());
        let embedder = Arc::new(MockEmbeddingProvider::new(384));
        let search_svc = SearchService::new(pool.clone(), embedder.clone());

        let note = note_svc
            .create(
                &alice,
                NewNote::new(
                    "Neural Networks",
                    "Backpropagation and gradient descent explained",
                ),
            )
            .await
            .expect("create note");

        // Manually index embedding chunk with mock embedder
        let emb = embedder
            .embed(&["Backpropagation and gradient descent explained".to_string()])
            .await
            .expect("mock embed")[0]
            .clone();

        repositories::knowledge::upsert_embedding_chunk(
            &pool,
            &alice,
            "note",
            note.id.0,
            0,
            "Backpropagation and gradient descent explained",
            &emb,
        )
        .await
        .expect("upsert embedding");

        // Search
        let results = search_svc
            .search(&alice, "Backpropagation gradient", None, 5)
            .await
            .expect("search results");

        assert!(!results.is_empty());
        assert_eq!(results[0].item_id, note.id.0);
        assert_eq!(results[0].title, "Neural Networks");
    }
}
