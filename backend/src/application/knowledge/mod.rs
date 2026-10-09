//! Knowledge base (notes, bookmarks, journal, footprints) application services.
//!
//! Owns business use cases, transactional boundaries, and outbox event
//! publication for notes and cross-domain semantic hybrid search.

use sqlx::PgPool;
use std::sync::Arc;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::error::KnowledgeError;
use crate::domain::knowledge::note::{
    normalize_tags, validate_content, validate_title, Note, NoteFormat,
};
use crate::domain::knowledge::search::{chunk_text, KnowledgeItemType, SearchResultItem};
use crate::infrastructure::inference::embedding::EmbeddingProvider;
use crate::infrastructure::postgres::repositories::{self, outbox};

impl From<KnowledgeError> for AppError {
    fn from(err: KnowledgeError) -> Self {
        match err {
            KnowledgeError::NotFound(id) => AppError::NotFound(format!("Note {id} not found")),
            other => AppError::Validation(other.to_string()),
        }
    }
}

/// Service managing user notes lifecycle and transactional outbox events.
#[derive(Clone)]
pub struct NoteService {
    pool: PgPool,
}

impl NoteService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Creates and persists a note within a transaction, emitting `SystemEvent::NoteCreated`.
    #[allow(clippy::too_many_arguments)]
    pub async fn create_note(
        &self,
        owner: &AccountId,
        title: &str,
        content: &str,
        format: Option<NoteFormat>,
        source_channel: Option<&str>,
        source_ref: Option<&str>,
        tags: Option<&[String]>,
    ) -> Result<Note, AppError> {
        let title = validate_title(title)?;
        let content = validate_content(content)?;
        let format = format.unwrap_or_default();
        let channel = source_channel.unwrap_or("web");
        let normalized_tags = tags.map(normalize_tags).unwrap_or_default();

        let mut tx = self.pool.begin().await?;
        let note = repositories::knowledge::insert_note(
            &mut *tx,
            owner,
            &title,
            &content,
            format.as_str(),
            channel,
            source_ref,
            &normalized_tags,
        )
        .await?;

        outbox::publish(&mut tx, &SystemEvent::NoteCreated(note.clone())).await?;
        let job = NewJob::knowledge_embed_item(*owner, KnowledgeItemType::Note, note.id.0);
        repositories::jobs::enqueue(&mut *tx, &job).await?;
        tx.commit().await?;

        Ok(note)
    }

    /// Updates note title, content, format, and tags, emitting `SystemEvent::NoteUpdated`.
    pub async fn update_note(
        &self,
        owner: &AccountId,
        id: i64,
        title: &str,
        content: &str,
        format: Option<NoteFormat>,
        tags: Option<&[String]>,
    ) -> Result<Note, AppError> {
        let title = validate_title(title)?;
        let content = validate_content(content)?;
        let format = format.unwrap_or_default();
        let normalized_tags = tags.map(normalize_tags).unwrap_or_default();

        let mut tx = self.pool.begin().await?;
        let note = repositories::knowledge::update_note(
            &mut *tx,
            id,
            owner,
            &title,
            &content,
            format.as_str(),
            &normalized_tags,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Note not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::NoteUpdated(note.clone())).await?;
        let job = NewJob::knowledge_embed_item(*owner, KnowledgeItemType::Note, note.id.0);
        repositories::jobs::enqueue(&mut *tx, &job).await?;
        tx.commit().await?;

        Ok(note)
    }

    /// Fetches a note by id for `owner`.
    pub async fn get_note(&self, owner: &AccountId, id: i64) -> Result<Note, AppError> {
        repositories::knowledge::get_note_by_id(&self.pool, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Note not found".to_string()))
    }

    /// Lists notes for `owner` with optional tag filter and pagination limit.
    pub async fn list_notes(
        &self,
        owner: &AccountId,
        tag: Option<&str>,
        include_archived: bool,
        limit: i64,
    ) -> Result<Vec<Note>, AppError> {
        let limit = limit.clamp(1, 100);
        let notes =
            repositories::knowledge::list_notes(&self.pool, owner, tag, include_archived, limit)
                .await?;
        Ok(notes)
    }

    /// Lists all unique tags used across an account's notes.
    pub async fn list_tags(&self, owner: &AccountId) -> Result<Vec<String>, AppError> {
        let tags = repositories::knowledge::list_all_tags(&self.pool, owner).await?;
        Ok(tags)
    }

    /// Toggles archived status for a note, emitting `SystemEvent::NoteUpdated`.
    pub async fn archive_note(
        &self,
        owner: &AccountId,
        id: i64,
        is_archived: bool,
    ) -> Result<Note, AppError> {
        let mut tx = self.pool.begin().await?;
        let success =
            repositories::knowledge::archive_note(&mut *tx, id, owner, is_archived).await?;
        if !success {
            return Err(AppError::NotFound("Note not found".to_string()));
        }
        let note = repositories::knowledge::get_note_by_id(&mut *tx, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Note not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::NoteUpdated(note.clone())).await?;
        tx.commit().await?;

        Ok(note)
    }

    /// Deletes a note and its embeddings, emitting `SystemEvent::NoteDeleted`.
    pub async fn delete_note(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let success = repositories::knowledge::delete_note(&mut *tx, id, owner).await?;
        if !success {
            return Err(AppError::NotFound("Note not found".to_string()));
        }
        repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, "note", id).await?;

        outbox::publish(
            &mut tx,
            &SystemEvent::NoteDeleted {
                id,
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;

        Ok(())
    }
}

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
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::infrastructure::inference::embedding::MockEmbeddingProvider;

    #[tokio::test]
    async fn test_note_service_crud_and_outbox_events() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let note_svc = NoteService::new(pool.clone());

        // Create
        let note = note_svc
            .create_note(
                &alice,
                "Atomic Habits",
                "Make it obvious, attractive, easy, satisfying.",
                Some(NoteFormat::Markdown),
                Some("matrix"),
                Some("!ref_msg_123"),
                Some(&["#Habits".to_string(), "productivity".to_string()]),
            )
            .await
            .expect("create note");

        assert_eq!(note.title, "Atomic Habits");
        assert_eq!(note.tags, vec!["habits", "productivity"]);

        // Read
        let fetched = note_svc
            .get_note(&alice, note.id.0)
            .await
            .expect("get note");
        assert_eq!(fetched.id, note.id);

        // Update
        let updated = note_svc
            .update_note(
                &alice,
                note.id.0,
                "Atomic Habits Revised",
                "Four laws of behavior change",
                None,
                Some(&["habits".to_string()]),
            )
            .await
            .expect("update note");
        assert_eq!(updated.title, "Atomic Habits Revised");
        assert_eq!(updated.tags, vec!["habits"]);

        // List
        let list = note_svc
            .list_notes(&alice, Some("habits"), false, 10)
            .await
            .expect("list notes");
        assert_eq!(list.len(), 1);

        // Archive
        let archived = note_svc
            .archive_note(&alice, note.id.0, true)
            .await
            .expect("archive note");
        assert!(archived.is_archived);

        let active_list = note_svc
            .list_notes(&alice, None, false, 10)
            .await
            .expect("list active notes");
        assert_eq!(active_list.len(), 0);

        // Delete
        note_svc
            .delete_note(&alice, note.id.0)
            .await
            .expect("delete note");

        let err = note_svc.get_note(&alice, note.id.0).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }

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
            .create_note(
                &alice,
                "Neural Networks",
                "Backpropagation and gradient descent explained",
                None,
                None,
                None,
                None,
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
