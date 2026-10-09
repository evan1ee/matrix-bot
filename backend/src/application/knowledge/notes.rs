//! Notes use cases: create, update, read, list, archive and delete, each in
//! one transaction that also writes the outbox event and enqueues indexing.
//!
//! Markdown is the only stored content. Everything else a note carries (hash,
//! plain text, excerpt) is derived here, on write, by [`markdown::derive`];
//! no caller computes it.

use sqlx::PgPool;

use super::markdown::{self, Derived};
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::SystemEvent;
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::error::KnowledgeError;
use crate::domain::knowledge::note::{
    normalize_tags, validate_content, validate_title, ExpectedHash, NewNote, Note, NoteDocument,
    NotePatch, NoteType,
};
use crate::domain::knowledge::search::KnowledgeItemType;
use crate::infrastructure::postgres::repositories::knowledge::NoteFields;
use crate::infrastructure::postgres::repositories::{self, outbox};

type Tx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// The user-editable state of a note after a patch or an append is applied.
struct Change {
    title: String,
    markdown: String,
    tags: Vec<String>,
    note_type: NoteType,
}

/// Service managing user notes lifecycle and transactional outbox events.
#[derive(Clone)]
pub struct NoteService {
    pool: PgPool,
}

fn not_found() -> AppError {
    AppError::NotFound("Note not found".to_string())
}

impl NoteService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Creates and persists a note within a transaction, emitting
    /// `SystemEvent::NoteCreated` and enqueueing its embedding job.
    pub async fn create(&self, owner: &AccountId, new: NewNote) -> Result<Note, AppError> {
        let title = validate_title(&new.title)?;
        let markdown = validate_content(&new.markdown)?;
        let tags = normalize_tags(&new.tags);
        let derived = markdown::derive(&markdown);

        let mut tx = self.pool.begin().await?;
        let note = repositories::knowledge::insert_note(
            &mut *tx,
            owner,
            &new.source_channel,
            new.source_ref.as_deref(),
            &fields(&title, &markdown, new.note_type, &tags, &derived),
        )
        .await?;

        outbox::publish(&mut tx, &SystemEvent::NoteCreated(note.clone())).await?;
        enqueue_embedding(&mut tx, owner, &note).await?;
        tx.commit().await?;

        Ok(note)
    }

    /// Applies `patch` under a row lock. Fails with `Conflict` when the patch
    /// expects a body hash that is no longer the stored one, so a stale editor
    /// cannot overwrite a newer write. A patch that changes nothing writes
    /// nothing: no `updated_at` bump, no event, no embedding job.
    pub async fn update(
        &self,
        owner: &AccountId,
        id: i64,
        patch: NotePatch,
    ) -> Result<Note, AppError> {
        let mut tx = self.pool.begin().await?;
        let current = lock(&mut tx, owner, id).await?;

        if let ExpectedHash::Exactly(expected) = &patch.expected {
            if *expected != current.note.content_hash {
                return Err(KnowledgeError::Conflict.into());
            }
        }

        let change = Change {
            title: match patch.title {
                Some(title) => validate_title(&title)?,
                None => current.note.title.clone(),
            },
            markdown: match patch.markdown {
                Some(markdown) => validate_content(&markdown)?,
                None => current.markdown.clone(),
            },
            tags: patch
                .tags
                .map(|tags| normalize_tags(&tags))
                .unwrap_or_else(|| current.note.tags.clone()),
            note_type: patch.note_type.unwrap_or(current.note.note_type),
        };
        let note = apply(&mut tx, owner, current, change).await?;
        tx.commit().await?;
        Ok(note)
    }

    /// Appends `addition` to the end of a note's Markdown after a blank line.
    /// Last writer wins: chat, MCP and LLM appends never carry a hash.
    pub async fn append(
        &self,
        owner: &AccountId,
        id: i64,
        addition: &str,
    ) -> Result<Note, AppError> {
        let addition = validate_content(addition)?;
        let mut tx = self.pool.begin().await?;
        let current = lock(&mut tx, owner, id).await?;
        let merged = format!(
            "{}\n\n{}",
            current.markdown.trim_end(),
            addition.trim_start()
        );
        let change = Change {
            title: current.note.title.clone(),
            markdown: validate_content(&merged)?,
            tags: current.note.tags.clone(),
            note_type: current.note.note_type,
        };
        let note = apply(&mut tx, owner, current, change).await?;
        tx.commit().await?;
        Ok(note)
    }

    /// Fetches a note with its Markdown for `owner`.
    pub async fn get(&self, owner: &AccountId, id: i64) -> Result<NoteDocument, AppError> {
        repositories::knowledge::get_note_document(&self.pool, id, owner)
            .await?
            .ok_or_else(not_found)
    }

    /// Lists note metadata for `owner` with an optional tag filter. Never
    /// loads a body.
    pub async fn list(
        &self,
        owner: &AccountId,
        tag: Option<&str>,
        include_archived: bool,
        limit: i64,
    ) -> Result<Vec<Note>, AppError> {
        let limit = limit.clamp(1, 100);
        Ok(
            repositories::knowledge::list_notes(&self.pool, owner, tag, include_archived, limit)
                .await?,
        )
    }

    /// Like [`Self::list`] but with bodies.
    ///
    /// Temporary: the REST list still returns `content` until the API
    /// contract changes (notes refactor task 7), which deletes this method.
    pub async fn list_documents(
        &self,
        owner: &AccountId,
        tag: Option<&str>,
        include_archived: bool,
        limit: i64,
    ) -> Result<Vec<NoteDocument>, AppError> {
        let limit = limit.clamp(1, 100);
        Ok(repositories::knowledge::list_note_documents(
            &self.pool,
            owner,
            tag,
            include_archived,
            limit,
        )
        .await?)
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
            return Err(not_found());
        }
        let note = repositories::knowledge::get_note_by_id(&mut *tx, id, owner)
            .await?
            .ok_or_else(not_found)?;

        outbox::publish(&mut tx, &SystemEvent::NoteUpdated(note.clone())).await?;
        tx.commit().await?;

        Ok(note)
    }

    /// Deletes a note and its embeddings, emitting `SystemEvent::NoteDeleted`.
    pub async fn delete_note(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let success = repositories::knowledge::delete_note(&mut *tx, id, owner).await?;
        if !success {
            return Err(not_found());
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

fn fields<'a>(
    title: &'a str,
    markdown: &'a str,
    note_type: NoteType,
    tags: &'a [String],
    derived: &'a Derived,
) -> NoteFields<'a> {
    NoteFields {
        title,
        content: markdown,
        note_type,
        tags,
        content_hash: &derived.content_hash,
        search_text: &derived.search_text,
        excerpt: &derived.excerpt,
    }
}

async fn lock(tx: &mut Tx<'_>, owner: &AccountId, id: i64) -> Result<NoteDocument, AppError> {
    repositories::knowledge::lock_note_for_update(&mut **tx, id, owner)
        .await?
        .ok_or_else(not_found)
}

async fn enqueue_embedding(
    tx: &mut Tx<'_>,
    owner: &AccountId,
    note: &Note,
) -> Result<(), AppError> {
    let job = NewJob::knowledge_embed_item(*owner, KnowledgeItemType::Note, note.id.0);
    repositories::jobs::enqueue(&mut **tx, &job).await?;
    Ok(())
}

/// Writes `change` over the locked `current` note, re-deriving the stored
/// columns. Re-embeds only when the title or body changed: tags and type do
/// not enter the embedding text.
async fn apply(
    tx: &mut Tx<'_>,
    owner: &AccountId,
    current: NoteDocument,
    change: Change,
) -> Result<Note, AppError> {
    let derived = markdown::derive(&change.markdown);
    let body_changed = derived.content_hash != current.note.content_hash;
    let title_changed = change.title != current.note.title;
    if !body_changed
        && !title_changed
        && change.tags == current.note.tags
        && change.note_type == current.note.note_type
    {
        return Ok(current.note);
    }

    let note = repositories::knowledge::update_note(
        &mut **tx,
        current.note.id.0,
        owner,
        &fields(
            &change.title,
            &change.markdown,
            change.note_type,
            &change.tags,
            &derived,
        ),
    )
    .await?
    .ok_or_else(not_found)?;

    outbox::publish(tx, &SystemEvent::NoteUpdated(note.clone())).await?;
    if body_changed || title_changed {
        enqueue_embedding(tx, owner, &note).await?;
    }
    Ok(note)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool, test_pool_sized};

    async fn setup(pool: &PgPool) -> (NoteService, AccountId) {
        run_migrations_unlocked(pool).await;
        let owner = AccountId::generate();
        seed_account(pool, &owner).await;
        (NoteService::new(pool.clone()), owner)
    }

    async fn embed_jobs(pool: &PgPool) -> i64 {
        sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM jobs WHERE job_type = 'knowledge.embed_item'"#
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn clear_jobs(pool: &PgPool) {
        sqlx::query!("DELETE FROM jobs")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn event_types(pool: &PgPool) -> Vec<String> {
        sqlx::query_scalar!("SELECT event_type FROM outbox_events ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn test_note_service_crud_and_outbox_events() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;

        // Create
        let note = svc
            .create(
                &alice,
                NewNote::new(
                    "Atomic Habits",
                    "Make it obvious, attractive, easy, satisfying.",
                )
                .source("matrix", Some("!ref_msg_123".to_string()))
                .tags(vec!["#Habits".to_string(), "productivity".to_string()]),
            )
            .await
            .expect("create note");

        assert_eq!(note.title, "Atomic Habits");
        assert_eq!(note.tags, vec!["habits", "productivity"]);
        assert_eq!(note.source_channel, "matrix");
        assert_eq!(note.source_ref.as_deref(), Some("!ref_msg_123"));

        // Read
        let fetched = svc.get(&alice, note.id.0).await.expect("get note");
        assert_eq!(fetched.note.id, note.id);
        assert_eq!(
            fetched.markdown,
            "Make it obvious, attractive, easy, satisfying."
        );

        // Update
        let updated = svc
            .update(
                &alice,
                note.id.0,
                NotePatch {
                    title: Some("Atomic Habits Revised".to_string()),
                    markdown: Some("Four laws of behavior change".to_string()),
                    tags: Some(vec!["habits".to_string()]),
                    ..NotePatch::default()
                },
            )
            .await
            .expect("update note");
        assert_eq!(updated.title, "Atomic Habits Revised");
        assert_eq!(updated.tags, vec!["habits"]);

        // List
        let list = svc
            .list(&alice, Some("habits"), false, 10)
            .await
            .expect("list notes");
        assert_eq!(list.len(), 1);

        // Archive
        let archived = svc
            .archive_note(&alice, note.id.0, true)
            .await
            .expect("archive note");
        assert!(archived.is_archived);
        assert!(svc.list(&alice, None, false, 10).await.unwrap().is_empty());

        // Delete
        svc.delete_note(&alice, note.id.0)
            .await
            .expect("delete note");
        let err = svc.get(&alice, note.id.0).await.unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));

        assert_eq!(
            event_types(&pool).await,
            vec!["NoteCreated", "NoteUpdated", "NoteUpdated", "NoteDeleted"]
        );
    }

    #[tokio::test]
    async fn create_stores_the_derived_columns_from_the_markdown() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;

        let note = svc
            .create(
                &alice,
                NewNote::new("Rust", "# Heading\n\nSome **bold** text"),
            )
            .await
            .unwrap();

        let row = sqlx::query!(
            "SELECT content_hash, search_text, excerpt FROM notes WHERE id = $1",
            note.id.0
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            row.content_hash.as_deref(),
            Some(markdown::content_hash("# Heading\n\nSome **bold** text").as_str())
        );
        assert_eq!(row.search_text.as_deref(), Some("Heading\nSome bold text"));
        assert_eq!(row.excerpt.as_deref(), Some("Heading Some bold text"));
        assert_eq!(note.excerpt, "Heading Some bold text");
    }

    #[tokio::test]
    async fn rejects_content_over_the_cap_and_unknown_state() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;

        let huge = "a".repeat(crate::domain::knowledge::note::MAX_CONTENT_CHARS + 1);
        let err = svc
            .create(&alice, NewNote::new("Big", huge))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
        assert!(event_types(&pool).await.is_empty());
    }

    #[tokio::test]
    async fn a_stale_expected_hash_is_a_conflict_and_changes_nothing() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;
        let note = svc
            .create(&alice, NewNote::new("T", "first"))
            .await
            .unwrap();
        let loaded_hash = note.content_hash.clone();

        // Someone else edits the note after the editor loaded it.
        svc.append(&alice, note.id.0, "second").await.unwrap();
        clear_jobs(&pool).await;

        let err = svc
            .update(
                &alice,
                note.id.0,
                NotePatch {
                    markdown: Some("stale overwrite".to_string()),
                    expected: ExpectedHash::Exactly(loaded_hash),
                    ..NotePatch::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");

        let stored = svc.get(&alice, note.id.0).await.unwrap();
        assert_eq!(stored.markdown, "first\n\nsecond");
        assert_eq!(
            embed_jobs(&pool).await,
            0,
            "a refused patch enqueues nothing"
        );
        assert_eq!(event_types(&pool).await, vec!["NoteCreated", "NoteUpdated"]);
    }

    #[tokio::test]
    async fn a_matching_expected_hash_applies_the_patch() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;
        let note = svc
            .create(&alice, NewNote::new("T", "first"))
            .await
            .unwrap();

        let updated = svc
            .update(
                &alice,
                note.id.0,
                NotePatch {
                    markdown: Some("second".to_string()),
                    expected: ExpectedHash::Exactly(note.content_hash.clone()),
                    ..NotePatch::default()
                },
            )
            .await
            .unwrap();
        assert_ne!(updated.content_hash, note.content_hash);
        assert_eq!(updated.content_hash, markdown::content_hash("second"));
    }

    #[tokio::test]
    async fn two_editors_with_the_same_hash_cannot_both_win() {
        let pool = test_pool_sized(6).await;
        let (svc, alice) = setup(&pool).await;
        let note = svc.create(&alice, NewNote::new("T", "base")).await.unwrap();

        let patch = |text: &str| NotePatch {
            markdown: Some(text.to_string()),
            expected: ExpectedHash::Exactly(note.content_hash.clone()),
            ..NotePatch::default()
        };
        let (a, b) = tokio::join!(
            svc.update(&alice, note.id.0, patch("from a")),
            svc.update(&alice, note.id.0, patch("from b")),
        );
        let outcomes = [a, b];
        let wins = outcomes.iter().filter(|r| r.is_ok()).count();
        let conflicts = outcomes
            .iter()
            .filter(|r| matches!(r, Err(AppError::Conflict(_))))
            .count();
        assert_eq!((wins, conflicts), (1, 1), "{outcomes:?}");
    }

    #[tokio::test]
    async fn concurrent_appends_are_serialised_and_none_is_lost() {
        let pool = test_pool_sized(6).await;
        let (svc, alice) = setup(&pool).await;
        let note = svc.create(&alice, NewNote::new("T", "base")).await.unwrap();

        let (a, b, c) = tokio::join!(
            svc.append(&alice, note.id.0, "one"),
            svc.append(&alice, note.id.0, "two"),
            svc.append(&alice, note.id.0, "three"),
        );
        a.unwrap();
        b.unwrap();
        c.unwrap();

        let markdown = svc.get(&alice, note.id.0).await.unwrap().markdown;
        for part in ["base", "one", "two", "three"] {
            assert!(markdown.contains(part), "lost {part}: {markdown:?}");
        }
    }

    #[tokio::test]
    async fn an_update_that_changes_nothing_writes_nothing() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;
        let note = svc.create(&alice, NewNote::new("T", "body")).await.unwrap();
        clear_jobs(&pool).await;

        let same = svc
            .update(
                &alice,
                note.id.0,
                NotePatch {
                    title: Some("T".to_string()),
                    markdown: Some("body".to_string()),
                    ..NotePatch::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(same, note, "updated_at is not bumped by a no-op");
        assert_eq!(embed_jobs(&pool).await, 0);
        assert_eq!(event_types(&pool).await, vec!["NoteCreated"]);
    }

    #[tokio::test]
    async fn embedding_is_enqueued_only_when_the_title_or_body_changes() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;
        let note = svc.create(&alice, NewNote::new("T", "body")).await.unwrap();

        // Tags and type are metadata: an event, but no re-embedding.
        clear_jobs(&pool).await;
        svc.update(
            &alice,
            note.id.0,
            NotePatch {
                tags: Some(vec!["rust".to_string()]),
                note_type: Some(NoteType::Idea),
                ..NotePatch::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(embed_jobs(&pool).await, 0);
        assert_eq!(event_types(&pool).await, vec!["NoteCreated", "NoteUpdated"]);

        // A title change re-embeds because the title is part of every chunk.
        svc.update(
            &alice,
            note.id.0,
            NotePatch {
                title: Some("New title".to_string()),
                ..NotePatch::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(embed_jobs(&pool).await, 1);

        clear_jobs(&pool).await;
        svc.update(
            &alice,
            note.id.0,
            NotePatch {
                markdown: Some("new body".to_string()),
                ..NotePatch::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(embed_jobs(&pool).await, 1);
    }

    #[tokio::test]
    async fn events_carry_metadata_only() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;
        // Longer than the excerpt, so the tail can only reach the event if the
        // whole body travels in it.
        let body = format!("{} TAILMARKER", "word ".repeat(100));
        svc.create(&alice, NewNote::new("Secret title", body))
            .await
            .unwrap();

        let payload = sqlx::query_scalar!("SELECT payload FROM outbox_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        let text = payload.to_string();
        assert!(!text.contains("TAILMARKER"), "{text}");
        let note = payload.get("payload").expect("event payload");
        assert!(note.get("content").is_none() && note.get("markdown").is_none());
        for key in ["excerpt", "content_hash", "note_type", "title"] {
            assert!(note.get(key).is_some(), "missing {key} in {text}");
        }
    }

    #[tokio::test]
    async fn a_failed_write_leaves_no_event_job_or_note() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;

        // A NUL byte is valid UTF-8 but PostgreSQL refuses it in TEXT, which
        // fails the transaction part-way through.
        let err = svc
            .create(&alice, NewNote::new("Bad", "has a \u{0} nul"))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Database(_)), "got {err:?}");

        assert!(event_types(&pool).await.is_empty());
        assert_eq!(embed_jobs(&pool).await, 0);
        assert!(svc.list(&alice, None, true, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn notes_are_invisible_and_immutable_to_other_owners() {
        let pool = test_pool().await;
        let (svc, alice) = setup(&pool).await;
        let bob = AccountId::generate();
        seed_account(&pool, &bob).await;
        let note = svc
            .create(&alice, NewNote::new("Mine", "private"))
            .await
            .unwrap();

        assert!(matches!(
            svc.get(&bob, note.id.0).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            svc.update(&bob, note.id.0, NotePatch::default()).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            svc.append(&bob, note.id.0, "x").await,
            Err(AppError::NotFound(_))
        ));
        assert!(svc.list(&bob, None, true, 10).await.unwrap().is_empty());
        assert_eq!(
            svc.get(&alice, note.id.0).await.unwrap().markdown,
            "private"
        );
    }
}
