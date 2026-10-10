//! Embedding of knowledge items and the backfill that keeps them current.
//!
//! Embeddings are derived data: dropping every chunk and the notes' indexed
//! markers loses nothing, because a backfill pass rebuilds them from the notes
//! and the other item tables.

use chrono::{DateTime, Utc};

use super::markdown;
use super::search::SearchService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::jobs::{JobType, NewJob};
use crate::domain::knowledge::search::{
    chunk_text, embedding_prefixes, IndexOutcome, KnowledgeItemType,
};
use crate::infrastructure::postgres::repositories;

const CHUNK_CHARS: usize = 500;
const CHUNK_OVERLAP: usize = 100;
/// Chunks per embedder request; the inference service refuses more than 128.
const EMBED_BATCH: usize = 64;
/// Pages one backfill run scans, so a huge backlog cannot hold a worker.
const MAX_BACKFILL_PAGES: i64 = 50;

fn external(err: impl std::fmt::Display) -> AppError {
    AppError::External {
        service: "embeddings",
        source: anyhow::anyhow!("{err}"),
    }
}

impl SearchService {
    /// Builds and stores the chunks for one item.
    ///
    /// A note whose chunks already come from its current title, body and the
    /// configured model is left alone. If a note changes while its chunks are
    /// being computed, nothing is written and the outcome is `Stale`: a newer
    /// job owns the work.
    pub async fn embed_item(
        &self,
        owner: &AccountId,
        item_type: KnowledgeItemType,
        item_id: i64,
    ) -> Result<IndexOutcome, AppError> {
        // Without an embedder there is nothing true to store; leave the item
        // unindexed so it is picked up once one is configured.
        if !self.active_embedder().is_available() {
            return Ok(IndexOutcome::Skipped);
        }
        match item_type {
            KnowledgeItemType::Note => self.embed_note(owner, item_id).await,
            other => self.embed_text_item(owner, other, item_id).await,
        }
    }

    async fn embed_note(&self, owner: &AccountId, id: i64) -> Result<IndexOutcome, AppError> {
        let kind = KnowledgeItemType::Note;
        let Some(input) =
            repositories::knowledge::get_note_for_indexing(&self.pool, owner, id).await?
        else {
            repositories::knowledge::delete_embeddings_for_item(
                &self.pool,
                owner,
                kind.as_str(),
                id,
            )
            .await?;
            return Ok(IndexOutcome::Removed);
        };

        let up_to_date = input.indexed_content_hash.as_deref() == Some(input.content_hash.as_str());
        if up_to_date
            && repositories::knowledge::chunks_match_model(&self.pool, owner, kind, id, &self.model)
                .await?
        {
            return Ok(IndexOutcome::Skipped);
        }

        let chunks = markdown::chunks(&input.title, &input.text);
        let embeddings = if chunks.is_empty() {
            Vec::new()
        } else {
            self.embed_passages(&chunks).await?
        };

        // Delete, insert and the staleness check commit together, or not at all.
        let mut tx = self.pool.begin().await?;
        repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, kind.as_str(), id)
            .await?;
        for (index, (chunk, embedding)) in chunks.iter().zip(&embeddings).enumerate() {
            repositories::knowledge::upsert_embedding_chunk(
                &mut *tx,
                owner,
                kind.as_str(),
                id,
                index as i32,
                chunk,
                embedding,
                &self.model,
            )
            .await?;
        }
        let current = repositories::knowledge::mark_note_indexed(
            &mut *tx,
            owner,
            id,
            &input.title,
            &input.content_hash,
        )
        .await?;
        if !current {
            return Ok(IndexOutcome::Stale);
        }
        tx.commit().await?;

        Ok(if chunks.is_empty() {
            IndexOutcome::Removed
        } else {
            IndexOutcome::Indexed
        })
    }

    async fn embed_text_item(
        &self,
        owner: &AccountId,
        kind: KnowledgeItemType,
        id: i64,
    ) -> Result<IndexOutcome, AppError> {
        let content =
            repositories::knowledge::get_item_content_for_embedding(&self.pool, owner, kind, id)
                .await?;
        let chunks = content
            .map(|text| chunk_text(&text, CHUNK_CHARS, CHUNK_OVERLAP))
            .unwrap_or_default();
        if chunks.is_empty() {
            repositories::knowledge::delete_embeddings_for_item(
                &self.pool,
                owner,
                kind.as_str(),
                id,
            )
            .await?;
            return Ok(IndexOutcome::Removed);
        }

        let embeddings = self.embed_passages(&chunks).await?;
        let mut tx = self.pool.begin().await?;
        repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, kind.as_str(), id)
            .await?;
        for (index, (chunk, embedding)) in chunks.iter().zip(&embeddings).enumerate() {
            repositories::knowledge::upsert_embedding_chunk(
                &mut *tx,
                owner,
                kind.as_str(),
                id,
                index as i32,
                chunk,
                embedding,
                &self.model,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(IndexOutcome::Indexed)
    }

    /// Embeds stored text. The model's passage prefix goes into the embedder's
    /// input only; the stored snippet stays as written.
    async fn embed_passages(&self, chunks: &[String]) -> Result<Vec<Vec<f32>>, AppError> {
        let embedder = self.active_embedder();
        let prefix = embedding_prefixes(&self.model).passage;
        let inputs: Vec<String> = chunks.iter().map(|c| format!("{prefix}{c}")).collect();
        // A 100,000-character note is hundreds of chunks; the inference service
        // bounds a request, so send them in slices.
        let mut embeddings = Vec::with_capacity(inputs.len());
        for slice in inputs.chunks(EMBED_BATCH) {
            embeddings.extend(embedder.embed(slice).await.map_err(external)?);
        }
        if embeddings.len() != chunks.len() {
            return Err(external(format!(
                "expected {} embeddings, got {}",
                chunks.len(),
                embeddings.len()
            )));
        }
        Ok(embeddings)
    }

    /// Enqueues low-priority embedding jobs for items whose embeddings are
    /// missing or out of date for the configured model. Each job is keyed by the
    /// version it embeds, so it is neither dropped because an earlier version's
    /// job finished nor repeated while it is pending.
    ///
    /// Items that already have a job for their version (queued, or failed and
    /// not yet purged) are not queued again, so the scan pages past them
    /// instead of re-reading the same head of the list every run. Returns how
    /// many jobs it queued; does nothing without an embedder.
    pub async fn backfill_embeddings(&self, page_size: i64) -> Result<usize, AppError> {
        if !self.active_embedder().is_available() {
            return Ok(0);
        }
        let page_size = page_size.max(1);
        let mut enqueued = 0;
        for page in 0..MAX_BACKFILL_PAGES {
            let items = repositories::knowledge::list_items_needing_index(
                &self.pool,
                &self.model,
                page_size,
                page * page_size,
            )
            .await?;
            let last_page = (items.len() as i64) < page_size;
            for item in items {
                let job = NewJob::knowledge_embed_item_versioned(
                    item.owner_id,
                    item.item_type,
                    item.item_id,
                    &item.version,
                )
                .with_priority(-1);
                if repositories::jobs::enqueue(&self.pool, &job)
                    .await?
                    .is_some()
                {
                    enqueued += 1;
                }
            }
            if last_page {
                break;
            }
        }
        Ok(enqueued)
    }

    /// The scheduler's hourly entry point: queues one backfill unless one is
    /// already waiting or running.
    pub async fn enqueue_backfill(&self, now: DateTime<Utc>) -> Result<usize, AppError> {
        let kind = JobType::KnowledgeBackfillEmbeddings.as_str();
        if repositories::jobs::has_open(&self.pool, kind).await? {
            return Ok(0);
        }
        let job = NewJob::knowledge_backfill_embeddings(now);
        Ok(usize::from(
            repositories::jobs::enqueue(&self.pool, &job)
                .await?
                .is_some(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use chrono::TimeZone;

    use super::*;
    use crate::application::knowledge::NoteService;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::db::DbPool;
    use crate::domain::jobs::KnowledgeEmbedPayload;
    use crate::domain::knowledge::note::{NewNote, NotePatch};
    use crate::infrastructure::inference::embedding::{
        EmbeddingError, EmbeddingProvider, LocalOnnxEmbeddingClient, MockEmbeddingProvider,
    };
    use crate::infrastructure::postgres::repositories::knowledge::{self as repo, ChunkRecord};

    /// Records every batch it was asked to embed, then answers like the mock.
    struct Recording {
        inner: MockEmbeddingProvider,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl Recording {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                inner: MockEmbeddingProvider::new(384),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EmbeddingProvider for Recording {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.calls.lock().unwrap().push(texts.to_vec());
            self.inner.embed(texts).await
        }
    }

    /// Edits a note the first time it is asked to embed, as a user would while
    /// a slow embedder is working.
    struct EditsDuringEmbed {
        inner: MockEmbeddingProvider,
        notes: NoteService,
        owner: AccountId,
        note_id: i64,
        fired: AtomicBool,
    }

    #[async_trait]
    impl EmbeddingProvider for EditsDuringEmbed {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            if !self.fired.swap(true, Ordering::SeqCst) {
                self.notes
                    .update(
                        &self.owner,
                        self.note_id,
                        NotePatch {
                            markdown: Some("edited while embedding".to_string()),
                            ..NotePatch::default()
                        },
                    )
                    .await
                    .expect("concurrent edit");
            }
            self.inner.embed(texts).await
        }
    }

    struct Fixture {
        pool: DbPool,
        notes: NoteService,
        search: SearchService,
        alice: AccountId,
        embedder: Arc<Recording>,
    }

    async fn fixture() -> Fixture {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;
        let embedder = Recording::new();
        Fixture {
            notes: NoteService::new(pool.clone()),
            search: SearchService::new(pool.clone(), embedder.clone()).with_model("model-a"),
            pool,
            alice,
            embedder,
        }
    }

    impl Fixture {
        async fn note(&self, title: &str, body: &str) -> i64 {
            self.notes
                .create(&self.alice, NewNote::new(title, body))
                .await
                .unwrap()
                .id
                .0
        }

        async fn embed(&self, id: i64) -> IndexOutcome {
            self.search
                .embed_item(&self.alice, KnowledgeItemType::Note, id)
                .await
                .unwrap()
        }

        async fn chunks(&self, id: i64) -> Vec<ChunkRecord> {
            repo::list_chunks(&self.pool, &self.alice, "note", id)
                .await
                .unwrap()
        }

        /// Forgets the jobs that creating items enqueued, so a test sees only
        /// the work the backfill itself queues.
        async fn clear_jobs(&self) {
            sqlx::query!("DELETE FROM jobs")
                .execute(&self.pool)
                .await
                .unwrap();
        }

        /// Runs every queued embedding job the way a worker would.
        async fn drain_embed_jobs(&self, service: &SearchService) -> usize {
            let mut ran = 0;
            while let Some(job) =
                repositories::jobs::claim(&self.pool, "test-worker", &["knowledge.embed_item"])
                    .await
                    .unwrap()
            {
                let payload: KnowledgeEmbedPayload = serde_json::from_value(job.payload).unwrap();
                service
                    .embed_item(&payload.account_id, payload.item_type, payload.item_id)
                    .await
                    .unwrap();
                repositories::jobs::complete(&self.pool, job.id, "test-worker")
                    .await
                    .unwrap();
                ran += 1;
            }
            ran
        }
    }

    #[tokio::test]
    async fn a_note_is_embedded_with_passage_prefixes_and_tagged_with_the_model() {
        let f = fixture().await;
        let e5 = f.search.with_model("multilingual-e5-small");
        let id = f.note("Rust", "ownership and borrowing").await;

        let outcome = e5
            .embed_item(&f.alice, KnowledgeItemType::Note, id)
            .await
            .unwrap();
        assert_eq!(outcome, IndexOutcome::Indexed);

        let chunks = f.chunks(id).await;
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].model, "multilingual-e5-small");
        assert_eq!(chunks[0].snippet, "Rust\n\nownership and borrowing");
        // The prefix is what the embedder saw, never what is stored or shown.
        assert_eq!(
            f.embedder.calls(),
            vec![vec!["passage: Rust\n\nownership and borrowing".to_string()]]
        );
        let input = repo::get_note_for_indexing(&f.pool, &f.alice, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            input.indexed_content_hash.as_deref(),
            Some(input.content_hash.as_str())
        );
    }

    #[tokio::test]
    async fn a_model_without_prefixes_embeds_the_bare_chunk() {
        let f = fixture().await;
        let id = f.note("Rust", "ownership").await;
        assert_eq!(f.embed(id).await, IndexOutcome::Indexed);
        assert_eq!(
            f.embedder.calls(),
            vec![vec!["Rust\n\nownership".to_string()]]
        );
    }

    #[tokio::test]
    async fn an_unchanged_note_is_not_embedded_again() {
        let f = fixture().await;
        let id = f.note("Rust", "ownership").await;
        assert_eq!(f.embed(id).await, IndexOutcome::Indexed);
        assert_eq!(f.embed(id).await, IndexOutcome::Skipped);
        assert_eq!(f.embedder.calls().len(), 1);
    }

    #[tokio::test]
    async fn an_edited_body_or_a_new_title_is_embedded_again() {
        let f = fixture().await;
        let id = f.note("Rust", "ownership").await;
        f.embed(id).await;

        f.notes
            .update(
                &f.alice,
                id,
                NotePatch {
                    markdown: Some("borrowing".to_string()),
                    ..NotePatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(f.embed(id).await, IndexOutcome::Indexed);
        assert_eq!(f.chunks(id).await[0].snippet, "Rust\n\nborrowing");

        f.notes
            .update(
                &f.alice,
                id,
                NotePatch {
                    title: Some("Rust lifetimes".to_string()),
                    ..NotePatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(f.embed(id).await, IndexOutcome::Indexed);
        assert_eq!(f.chunks(id).await[0].snippet, "Rust lifetimes\n\nborrowing");
        assert_eq!(f.embedder.calls().len(), 3);
    }

    #[tokio::test]
    async fn markdown_syntax_is_not_embedded() {
        let f = fixture().await;
        let id = f
            .note(
                "Doc",
                "# Heading\n\nsome **bold** [link](https://example.com)",
            )
            .await;
        f.embed(id).await;
        assert_eq!(
            f.chunks(id).await[0].snippet,
            "Doc\n\nHeading\nsome bold link"
        );
    }

    #[tokio::test]
    async fn a_stale_embed_loses_to_the_newer_write() {
        let f = fixture().await;
        let id = f.note("Rust", "original body").await;
        let racing = f.search.with_embedder(Arc::new(EditsDuringEmbed {
            inner: MockEmbeddingProvider::new(384),
            notes: f.notes.clone(),
            owner: f.alice,
            note_id: id,
            fired: AtomicBool::new(false),
        }));

        let outcome = racing
            .embed_item(&f.alice, KnowledgeItemType::Note, id)
            .await
            .unwrap();
        assert_eq!(outcome, IndexOutcome::Stale);
        assert!(f.chunks(id).await.is_empty(), "stale chunks are not kept");
        let input = repo::get_note_for_indexing(&f.pool, &f.alice, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(input.indexed_content_hash, None);

        // The retry the handler schedules then indexes the edited body.
        let retry = racing
            .embed_item(&f.alice, KnowledgeItemType::Note, id)
            .await
            .unwrap();
        assert_eq!(retry, IndexOutcome::Indexed);
        assert_eq!(
            f.chunks(id).await[0].snippet,
            "Rust\n\nedited while embedding"
        );
    }

    #[tokio::test]
    async fn switching_models_re_embeds_every_chunk_under_the_new_model() {
        let f = fixture().await;
        let id = f.note("Rust", "ownership").await;
        f.embed(id).await;
        assert!(f.chunks(id).await.iter().all(|c| c.model == "model-a"));

        f.clear_jobs().await;
        let b = f.search.with_model("model-b");
        // The backfill finds the note even though its body is unchanged...
        assert_eq!(b.backfill_embeddings(10).await.unwrap(), 1);
        // ...and the embed does not skip it.
        assert_eq!(f.drain_embed_jobs(&b).await, 1);
        let chunks = f.chunks(id).await;
        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| c.model == "model-b"));
        assert_eq!(f.embedder.calls().len(), 2);

        // Nothing is left to do under the new model.
        assert_eq!(b.backfill_embeddings(10).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn dropping_every_embedding_loses_nothing() {
        let f = fixture().await;
        let long_body = "A sentence about tokio and async runtimes. ".repeat(40);
        let first = f.note("Long", &long_body).await;
        let second = f.note("Short", "# Title\n\ntext with **markup**").await;
        let bookmark = sqlx::query_scalar!(
            r#"INSERT INTO bookmarks (owner_id, url, title, domain, description)
               VALUES ($1, 'https://example.com/b', 'Saved', 'example.com', 'about kayaks')
               RETURNING id"#,
            f.alice.as_uuid()
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();

        f.clear_jobs().await;

        // Build everything once, the normal way.
        assert_eq!(f.search.backfill_embeddings(100).await.unwrap(), 3);
        assert_eq!(f.drain_embed_jobs(&f.search).await, 3);
        let before_first = f.chunks(first).await;
        let before_second = f.chunks(second).await;
        let before_bookmark = repo::list_chunks(&f.pool, &f.alice, "bookmark", bookmark)
            .await
            .unwrap();
        assert!(before_first.len() > 1, "the long note spans several chunks");
        assert!(!before_bookmark.is_empty());

        // Lose every embedding byte; the note columns are NOT NULL and stay.
        sqlx::query!("DELETE FROM knowledge_embeddings")
            .execute(&f.pool)
            .await
            .unwrap();
        sqlx::query!("UPDATE notes SET indexed_content_hash = NULL, indexed_at = NULL")
            .execute(&f.pool)
            .await
            .unwrap();
        f.clear_jobs().await;

        // Rebuild from the notes and the bookmark alone.
        assert_eq!(f.search.backfill_embeddings(100).await.unwrap(), 3);
        assert_eq!(f.drain_embed_jobs(&f.search).await, 3);

        assert_eq!(f.chunks(first).await, before_first);
        assert_eq!(f.chunks(second).await, before_second);
        let after_bookmark = repo::list_chunks(&f.pool, &f.alice, "bookmark", bookmark)
            .await
            .unwrap();
        assert_eq!(after_bookmark, before_bookmark);
    }

    #[tokio::test]
    async fn an_edit_is_re_embedded_even_when_the_first_job_has_finished() {
        let f = fixture().await;
        let id = f.note("Rust", "v1").await;
        assert_eq!(f.drain_embed_jobs(&f.search).await, 1);

        f.notes
            .update(
                &f.alice,
                id,
                NotePatch {
                    markdown: Some("v2".to_string()),
                    ..NotePatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(
            f.drain_embed_jobs(&f.search).await,
            1,
            "the edit is not swallowed"
        );
        assert_eq!(f.chunks(id).await[0].snippet, "Rust\n\nv2");
    }

    #[tokio::test]
    async fn a_note_with_no_text_is_cleared_and_not_requeued_forever() {
        let f = fixture().await;
        // An image with no alt text has no plain text to embed.
        let id = f.note("Pic", "![](photo.png)").await;
        assert_eq!(f.embed(id).await, IndexOutcome::Removed);
        assert!(f.chunks(id).await.is_empty());
        assert_eq!(f.search.backfill_embeddings(10).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_missing_note_clears_its_embeddings() {
        let f = fixture().await;
        let id = f.note("Gone", "text").await;
        f.embed(id).await;
        f.notes.delete_note(&f.alice, id).await.unwrap();
        // delete_note already purges; plant a stale chunk to prove embed does too.
        repo::upsert_embedding_chunk(
            &f.pool,
            &f.alice,
            "note",
            id,
            0,
            "stale",
            &vec![0.0; 384],
            "model-a",
        )
        .await
        .unwrap();
        assert_eq!(f.embed(id).await, IndexOutcome::Removed);
        assert!(f.chunks(id).await.is_empty());
    }

    #[tokio::test]
    async fn other_item_types_get_prefixes_and_the_model_tag() {
        let f = fixture().await;
        let e5 = f.search.with_model("multilingual-e5-small");
        let bookmark = sqlx::query_scalar!(
            r#"INSERT INTO bookmarks (owner_id, url, title, domain)
               VALUES ($1, 'https://example.com/b', 'Saved page', 'example.com') RETURNING id"#,
            f.alice.as_uuid()
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();

        let outcome = e5
            .embed_item(&f.alice, KnowledgeItemType::Bookmark, bookmark)
            .await
            .unwrap();
        assert_eq!(outcome, IndexOutcome::Indexed);
        let chunks = repo::list_chunks(&f.pool, &f.alice, "bookmark", bookmark)
            .await
            .unwrap();
        assert_eq!(chunks[0].model, "multilingual-e5-small");
        assert_eq!(chunks[0].snippet, "Saved page");
        assert_eq!(
            f.embedder.calls(),
            vec![vec!["passage: Saved page".to_string()]]
        );
    }

    #[tokio::test]
    async fn the_hourly_backfill_is_queued_once_per_hour_and_never_while_open() {
        let f = fixture().await;
        let at = |h, m| Utc.with_ymd_and_hms(2026, 10, 9, h, m, 0).unwrap();

        assert_eq!(f.search.enqueue_backfill(at(10, 0)).await.unwrap(), 1);
        assert_eq!(
            f.search.enqueue_backfill(at(10, 5)).await.unwrap(),
            0,
            "already open"
        );

        // The run finishes; the same hour must not queue it again, the next must.
        sqlx::query!("UPDATE jobs SET status = 'completed', completed_at = NOW()")
            .execute(&f.pool)
            .await
            .unwrap();
        assert_eq!(f.search.enqueue_backfill(at(10, 40)).await.unwrap(), 0);
        assert_eq!(f.search.enqueue_backfill(at(11, 0)).await.unwrap(), 1);
    }

    /// Accepts at most `limit` inputs per call, like the inference service.
    struct BatchLimited {
        inner: MockEmbeddingProvider,
        limit: usize,
    }

    #[async_trait]
    impl EmbeddingProvider for BatchLimited {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            if texts.len() > self.limit {
                return Err(EmbeddingError::Upstream {
                    status: 400,
                    message: format!("At most {} inputs per request", self.limit),
                });
            }
            self.inner.embed(texts).await
        }
    }

    #[tokio::test]
    async fn a_note_with_more_chunks_than_one_request_allows_is_still_embedded() {
        let f = fixture().await;
        let search = SearchService::new(
            f.pool.clone(),
            Arc::new(BatchLimited {
                inner: MockEmbeddingProvider::new(384),
                limit: EMBED_BATCH,
            }),
        )
        .with_model("model-a");
        // Within the 100,000-character cap, but far more than EMBED_BATCH chunks.
        let body = "A sentence about tokio and async runtimes. ".repeat(1_400);
        let id = f.note("Long", &body).await;

        let outcome = search
            .embed_item(&f.alice, KnowledgeItemType::Note, id)
            .await
            .unwrap();

        assert_eq!(outcome, IndexOutcome::Indexed);
        assert!(f.chunks(id).await.len() > EMBED_BATCH);
    }

    #[tokio::test]
    async fn an_unconfigured_embedder_indexes_nothing_and_queues_nothing() {
        let f = fixture().await;
        let id = f.note("Rust", "a note").await;
        f.clear_jobs().await;
        let search = SearchService::new(
            f.pool.clone(),
            Arc::new(LocalOnnxEmbeddingClient::new(None, None)),
        )
        .with_model("model-a");

        assert_eq!(search.backfill_embeddings(10).await.unwrap(), 0);
        let outcome = search
            .embed_item(&f.alice, KnowledgeItemType::Note, id)
            .await
            .unwrap();

        assert_eq!(outcome, IndexOutcome::Skipped);
        assert!(f.chunks(id).await.is_empty(), "no fake vectors are stored");
        let indexed =
            sqlx::query_scalar!("SELECT indexed_content_hash FROM notes WHERE id = $1", id)
                .fetch_one(&f.pool)
                .await
                .unwrap();
        assert_eq!(indexed, None, "the note still counts as needing an index");
        // Once an embedder exists the same note is picked up.
        assert_eq!(f.search.backfill_embeddings(10).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn the_backfill_queues_new_work_past_items_that_already_have_jobs() {
        let f = fixture().await;
        for n in 0..2 {
            f.note("Old", &format!("old {n}")).await;
        }
        f.clear_jobs().await;
        assert_eq!(f.search.backfill_embeddings(2).await.unwrap(), 2);
        // The two oldest items keep their (say, failed) jobs, so a fixed first
        // page of two would never reach the new ones.
        for n in 0..3 {
            f.note("New", &format!("new {n}")).await;
        }

        assert_eq!(f.search.backfill_embeddings(2).await.unwrap(), 3);
        assert_eq!(f.search.backfill_embeddings(2).await.unwrap(), 0);
    }
}
