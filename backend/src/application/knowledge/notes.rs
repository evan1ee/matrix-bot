//! Notes use cases: create, update, read, list, archive and delete, each in
//! one transaction that also writes the outbox event and enqueues indexing.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::note::{
    normalize_tags, validate_content, validate_title, Note, NoteFormat,
};
use crate::domain::knowledge::search::KnowledgeItemType;
use crate::infrastructure::postgres::repositories::{self, outbox};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};

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
}
