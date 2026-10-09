//! Background durable jobs for the knowledge base embeddings.
//!
//! Handlers:
//! - `knowledge.embed_item`: fetches item content, chunks text with overlap,
//!   computes embeddings, and upserts chunks into `knowledge_embeddings`.
//! - `knowledge.backfill_embeddings`: scans unindexed items across notes,
//!   bookmarks, journals, and footprints, enqueuing low-priority embed jobs.

use async_trait::async_trait;

use super::parse_payload;
use crate::application::AppServices;
use crate::domain::jobs::{JobType, KnowledgeEmbedPayload};
use crate::jobs::{JobContext, JobError, JobHandler};

/// Handler for `knowledge.embed_item`.
pub struct KnowledgeEmbedHandler {
    services: AppServices,
}

impl KnowledgeEmbedHandler {
    pub fn new(services: AppServices) -> Self {
        Self { services }
    }
}

#[async_trait]
impl JobHandler for KnowledgeEmbedHandler {
    fn job_type(&self) -> JobType {
        JobType::KnowledgeEmbedItem
    }

    async fn run(&self, _ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError> {
        let p: KnowledgeEmbedPayload = parse_payload(payload)?;
        self.services
            .knowledge_search
            .embed_item(&p.account_id, p.item_type, p.item_id)
            .await
            .map_err(|e| JobError::Permanent(anyhow::anyhow!("{e}")))?;
        Ok(())
    }
}

/// Handler for `knowledge.backfill_embeddings`.
pub struct KnowledgeBackfillHandler {
    services: AppServices,
}

impl KnowledgeBackfillHandler {
    pub fn new(services: AppServices) -> Self {
        Self { services }
    }
}

#[async_trait]
impl JobHandler for KnowledgeBackfillHandler {
    fn job_type(&self) -> JobType {
        JobType::KnowledgeBackfillEmbeddings
    }

    async fn run(&self, _ctx: &JobContext, _payload: serde_json::Value) -> Result<(), JobError> {
        self.services
            .knowledge_search
            .backfill_embeddings(50)
            .await
            .map_err(|e| JobError::Permanent(anyhow::anyhow!("{e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::domain::accounts::AccountId;
    use crate::domain::jobs::JobId;
    use crate::domain::knowledge::note::NewNote;
    use crate::infrastructure::inference::embedding::MockEmbeddingProvider;
    use crate::infrastructure::postgres::repositories;

    fn job_ctx(owner: Option<AccountId>) -> JobContext {
        JobContext {
            job_id: JobId(1),
            account_id: owner,
            attempt: 1,
            max_attempts: 3,
            cancel: CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn test_knowledge_embed_item_handler_flow() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let mock_embedder = Arc::new(MockEmbeddingProvider::new(384));
        let mut services = AppServices::new(&pool);
        services.knowledge_search = services.knowledge_search.with_embedder(mock_embedder);

        // Create a note
        let note = services
            .notes
            .create(
                &alice,
                NewNote::new(
                    "Microservices vs Modular Monolith",
                    "Start from the smallest version that works end to end.",
                ),
            )
            .await
            .expect("create note");

        let handler = KnowledgeEmbedHandler::new(services.clone());
        let payload = serde_json::json!({
            "account_id": alice,
            "item_type": "note",
            "item_id": note.id.0,
        });

        // Run handler
        let res = handler.run(&job_ctx(Some(alice)), payload).await;
        assert!(res.is_ok(), "handler failed: {res:?}");

        // Verify search finds it through generated embedding
        let results = services
            .knowledge_search
            .search(&alice, "Modular Monolith", None, 5)
            .await
            .expect("search");
        assert!(!results.is_empty());
        assert_eq!(results[0].item_id, note.id.0);
        assert_eq!(results[0].title, "Microservices vs Modular Monolith");
    }

    #[tokio::test]
    async fn test_knowledge_backfill_handler_enqueues_unindexed() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let mock_embedder = Arc::new(MockEmbeddingProvider::new(384));
        let mut services = AppServices::new(&pool);
        services.knowledge_search = services.knowledge_search.with_embedder(mock_embedder);

        // Directly insert note into database without enqueuing job or embeddings
        let note = repositories::knowledge::insert_test_note(
            &pool,
            &alice,
            "Unindexed Direct Note",
            "Content that needs backfill",
            &[],
        )
        .await
        .expect("direct insert note");

        let handler = KnowledgeBackfillHandler::new(services.clone());
        let res = handler.run(&job_ctx(None), serde_json::json!({})).await;
        assert!(res.is_ok(), "backfill run failed: {res:?}");

        // Verify job was enqueued in jobs table
        let claimed = repositories::jobs::claim(&pool, "worker-test", &["knowledge.embed_item"])
            .await
            .expect("claim jobs")
            .expect("job should be claimed");

        assert_eq!(claimed.job_type, "knowledge.embed_item");
        assert_eq!(claimed.payload["item_id"], note.id.0);
        assert_eq!(claimed.payload["item_type"], "note");
    }
}
