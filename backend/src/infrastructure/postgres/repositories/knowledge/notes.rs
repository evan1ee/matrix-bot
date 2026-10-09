//! PostgreSQL repository for notes CRUD operations.
//!
//! `NoteRow` is the metadata row that lists, events and chat lists carry; it
//! has no `content` field on purpose, so no list can load a body by accident.
//! `NoteDocumentRow` adds the Markdown for the few reads that need it.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::{ContentHash, Note, NoteDocument, NoteId, NoteType};

/// The derived and user-editable columns written together on every create and
/// update. The derived values (`content_hash`, `search_text`, `excerpt`) are
/// computed by the service; the repository only stores them.
pub struct NoteFields<'a> {
    pub title: &'a str,
    pub content: &'a str,
    pub note_type: NoteType,
    pub tags: &'a [String],
    pub content_hash: &'a ContentHash,
    pub search_text: &'a str,
    pub excerpt: &'a str,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct NoteRow {
    pub id: i64,
    pub owner_id: uuid::Uuid,
    pub title: String,
    pub note_type: String,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub excerpt: String,
    pub content_hash: String,
    pub is_archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub indexed_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct NoteDocumentRow {
    pub id: i64,
    pub owner_id: uuid::Uuid,
    pub title: String,
    pub content: String,
    pub note_type: String,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub excerpt: String,
    pub content_hash: String,
    pub is_archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub indexed_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn decode_error(err: impl std::error::Error + Send + Sync + 'static) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(err))
}

impl TryFrom<NoteRow> for Note {
    type Error = sqlx::Error;

    fn try_from(row: NoteRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: NoteId(row.id),
            owner_id: AccountId::from_uuid(row.owner_id),
            title: row.title,
            note_type: row.note_type.parse().map_err(decode_error)?,
            source_channel: row.source_channel,
            source_ref: row.source_ref,
            tags: row.tags,
            excerpt: row.excerpt,
            content_hash: ContentHash::from_hex(&row.content_hash).map_err(decode_error)?,
            is_archived: row.is_archived,
            created_at: row.created_at,
            updated_at: row.updated_at,
            indexed_at: row.indexed_at,
        })
    }
}

impl TryFrom<NoteDocumentRow> for NoteDocument {
    type Error = sqlx::Error;

    fn try_from(row: NoteDocumentRow) -> Result<Self, Self::Error> {
        let markdown = row.content;
        let note = Note::try_from(NoteRow {
            id: row.id,
            owner_id: row.owner_id,
            title: row.title,
            note_type: row.note_type,
            source_channel: row.source_channel,
            source_ref: row.source_ref,
            tags: row.tags,
            excerpt: row.excerpt,
            content_hash: row.content_hash,
            is_archived: row.is_archived,
            created_at: row.created_at,
            updated_at: row.updated_at,
            indexed_at: row.indexed_at,
        })?;
        Ok(Self { note, markdown })
    }
}

pub async fn insert_note(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    source_channel: &str,
    source_ref: Option<&str>,
    fields: &NoteFields<'_>,
) -> Result<Note, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        INSERT INTO notes (
            owner_id, title, content, note_type, source_channel, source_ref, tags,
            content_hash, search_text, excerpt
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING
            id, owner_id, title, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        "#,
        owner.as_uuid(),
        fields.title,
        fields.content,
        fields.note_type.as_str(),
        source_channel,
        source_ref,
        fields.tags,
        fields.content_hash.as_str(),
        fields.search_text,
        fields.excerpt
    )
    .fetch_one(executor)
    .await?;

    Note::try_from(row)
}

pub async fn update_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
    fields: &NoteFields<'_>,
) -> Result<Option<Note>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        UPDATE notes
        SET
            -- The title is part of every embedded chunk, so a renamed note's
            -- embeddings are stale even though its body hash is unchanged.
            indexed_content_hash = CASE WHEN title IS DISTINCT FROM $3 THEN NULL
                                        ELSE indexed_content_hash END,
            title = $3,
            content = $4,
            note_type = $5,
            tags = $6,
            content_hash = $7,
            search_text = $8,
            excerpt = $9,
            updated_at = NOW()
        WHERE id = $1 AND owner_id = $2
        RETURNING
            id, owner_id, title, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        "#,
        id,
        owner.as_uuid(),
        fields.title,
        fields.content,
        fields.note_type.as_str(),
        fields.tags,
        fields.content_hash.as_str(),
        fields.search_text,
        fields.excerpt
    )
    .fetch_optional(executor)
    .await?;

    row.map(Note::try_from).transpose()
}

pub async fn get_note_by_id(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Note>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        SELECT
            id, owner_id, title, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(executor)
    .await?;

    row.map(Note::try_from).transpose()
}

pub async fn get_note_document(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<NoteDocument>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteDocumentRow,
        r#"
        SELECT
            id, owner_id, title, content, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(executor)
    .await?;

    row.map(NoteDocument::try_from).transpose()
}

/// Every note of `owner` with its body, archived ones included, oldest id
/// first. For export: the caller passes a cap and treats a full page as
/// "maybe more".
pub async fn list_documents(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    limit: i64,
) -> Result<Vec<NoteDocument>, sqlx::Error> {
    let rows = sqlx::query_as!(
        NoteDocumentRow,
        r#"
        SELECT
            id, owner_id, title, content, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        FROM notes
        WHERE owner_id = $1
        ORDER BY id
        LIMIT $2
        "#,
        owner.as_uuid(),
        limit
    )
    .fetch_all(executor)
    .await?;

    rows.into_iter().map(NoteDocument::try_from).collect()
}

/// Loads a note with its body and locks the row until the transaction ends, so
/// a read-modify-write on the body cannot interleave with another writer.
pub async fn lock_note_for_update(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<NoteDocument>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteDocumentRow,
        r#"
        SELECT
            id, owner_id, title, content, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        FROM notes
        WHERE id = $1 AND owner_id = $2
        FOR UPDATE
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(executor)
    .await?;

    row.map(NoteDocument::try_from).transpose()
}

/// Metadata only: this query never selects `content`.
///
/// `search` is plain text matched as a case-insensitive substring of the
/// title or body (wildcards escaped), the same expression the `pg_trgm` index
/// on `notes` covers.
pub async fn list_notes(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    tag_filter: Option<&str>,
    search: Option<&str>,
    include_archived: bool,
    limit: i64,
) -> Result<Vec<Note>, sqlx::Error> {
    let search_pattern = search.map(super::retrieval::like_pattern);
    let rows = sqlx::query_as!(
        NoteRow,
        r#"
        SELECT
            id, owner_id, title, note_type, source_channel, source_ref, tags,
            excerpt, content_hash,
            is_archived, created_at, updated_at, indexed_at
        FROM notes
        WHERE owner_id = $1
          AND ($2::TEXT IS NULL OR $2 = ANY(tags))
          AND ($3::BOOLEAN OR is_archived = FALSE)
          AND ($5::TEXT IS NULL OR (title || ' ' || search_text) ILIKE $5)
        ORDER BY updated_at DESC
        LIMIT $4
        "#,
        owner.as_uuid(),
        tag_filter,
        include_archived,
        limit,
        search_pattern
    )
    .fetch_all(executor)
    .await?;

    rows.into_iter().map(Note::try_from).collect()
}

pub async fn archive_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
    is_archived: bool,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        UPDATE notes
        SET is_archived = $3, updated_at = NOW()
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid(),
        is_archived
    )
    .execute(executor)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn delete_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        DELETE FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .execute(executor)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn list_all_tags(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT DISTINCT unnest(tags) AS "tag!"
        FROM notes
        WHERE owner_id = $1 AND is_archived = FALSE
        ORDER BY "tag!" ASC
        "#,
        owner.as_uuid()
    )
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(|r| r.tag).collect())
}

/// Inserts a web note whose derived columns are the body itself, for tests in
/// other modules that only need a row to exist.
#[cfg(test)]
pub async fn insert_test_note(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    title: &str,
    content: &str,
    tags: &[String],
) -> Result<Note, sqlx::Error> {
    use sha2::{Digest, Sha256};
    let hash = ContentHash::from_hex(&format!("{:x}", Sha256::digest(content.as_bytes())))
        .expect("sha256 hex");
    insert_note(
        executor,
        owner,
        "web",
        None,
        &NoteFields {
            title,
            content,
            note_type: NoteType::Note,
            tags,
            content_hash: &hash,
            search_text: content,
            excerpt: content,
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};

    // SHA-256 of "abc".
    const ABC_HASH: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn fields<'a>(
        title: &'a str,
        content: &'a str,
        tags: &'a [String],
        hash: &'a ContentHash,
    ) -> NoteFields<'a> {
        NoteFields {
            title,
            content,
            note_type: NoteType::Note,
            tags,
            content_hash: hash,
            search_text: content,
            excerpt: content,
        }
    }

    #[tokio::test]
    async fn test_note_crud_and_isolation() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        let bob = AccountId::generate();
        seed_account(&pool, &alice).await;
        seed_account(&pool, &bob).await;

        let hash = ContentHash::from_hex(ABC_HASH).unwrap();
        let tags = ["productivity".to_string(), "focus".to_string()];
        let note = insert_note(
            &pool,
            &alice,
            "web",
            None,
            &fields("Deep Work", "Focus without distraction", &tags, &hash),
        )
        .await
        .expect("insert note");

        assert_eq!(note.title, "Deep Work");
        assert_eq!(note.tags, vec!["productivity", "focus"]);
        assert_eq!(note.owner_id, alice);
        assert_eq!(note.note_type, NoteType::Note);
        assert_eq!(note.excerpt, "Focus without distraction");
        assert_eq!(note.content_hash, hash);
        assert!(note.indexed_at.is_none());

        // Bob cannot read Alice's note, by any read path
        assert!(get_note_by_id(&pool, note.id.0, &bob)
            .await
            .expect("query note")
            .is_none());
        assert!(get_note_document(&pool, note.id.0, &bob)
            .await
            .expect("query document")
            .is_none());
        assert!(lock_note_for_update(&pool, note.id.0, &bob)
            .await
            .expect("lock")
            .is_none());
        let bob_update = update_note(&pool, note.id.0, &bob, &fields("Hijack", "x", &[], &hash))
            .await
            .expect("update as bob");
        assert!(
            bob_update.is_none(),
            "another owner's note is not updatable"
        );
        assert!(!delete_note(&pool, note.id.0, &bob).await.unwrap());

        // Alice reads her note, with and without the body
        let alice_read = get_note_by_id(&pool, note.id.0, &alice)
            .await
            .expect("query note")
            .expect("note found");
        assert_eq!(alice_read.id, note.id);
        let document = get_note_document(&pool, note.id.0, &alice)
            .await
            .expect("query document")
            .expect("document found");
        assert_eq!(document.markdown, "Focus without distraction");
        assert_eq!(document.note, alice_read);

        // Update note
        let new_tags = ["focus".to_string()];
        let updated = update_note(
            &pool,
            note.id.0,
            &alice,
            &NoteFields {
                note_type: NoteType::Idea,
                ..fields("Deep Work 2.0", "Updated focus tactics", &new_tags, &hash)
            },
        )
        .await
        .expect("update note")
        .expect("note returned");
        assert_eq!(updated.title, "Deep Work 2.0");
        assert_eq!(updated.tags, vec!["focus"]);
        assert_eq!(updated.note_type, NoteType::Idea);
        assert_eq!(updated.excerpt, "Updated focus tactics");

        // List notes with tag filter
        let list_filtered = list_notes(&pool, &alice, Some("focus"), None, false, 10)
            .await
            .expect("list notes");
        assert_eq!(list_filtered.len(), 1);

        let list_unmatched = list_notes(&pool, &alice, Some("nonexistent"), None, false, 10)
            .await
            .expect("list notes");
        assert_eq!(list_unmatched.len(), 0);

        // Archive note
        let archived = archive_note(&pool, note.id.0, &alice, true)
            .await
            .expect("archive note");
        assert!(archived);

        let list_active = list_notes(&pool, &alice, None, None, false, 10)
            .await
            .expect("list active");
        assert_eq!(list_active.len(), 0);

        let list_all = list_notes(&pool, &alice, None, None, true, 10)
            .await
            .expect("list all");
        assert_eq!(list_all.len(), 1);

        // Delete note
        let deleted = delete_note(&pool, note.id.0, &alice)
            .await
            .expect("delete note");
        assert!(deleted);

        let read_deleted = get_note_by_id(&pool, note.id.0, &alice)
            .await
            .expect("query note");
        assert!(read_deleted.is_none());
    }

    #[tokio::test]
    async fn the_derived_columns_are_required_by_the_schema() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let err = sqlx::query!(
            "INSERT INTO notes (owner_id, title, content) VALUES ($1, 'Bare', 'abc')",
            alice.as_uuid()
        )
        .execute(&pool)
        .await
        .expect_err("a note without content_hash, search_text and excerpt is refused");
        assert!(
            err.to_string().contains("null value"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn the_search_vector_is_generated_from_the_derived_text_and_title() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let hash = ContentHash::from_hex(ABC_HASH).unwrap();
        let fields = NoteFields {
            search_text: "plain words only",
            ..fields("Zebra title", "# raw **markdown** syntax", &[], &hash)
        };
        let note = insert_note(&pool, &alice, "web", None, &fields)
            .await
            .unwrap();

        let matched = sqlx::query_scalar!(
            r#"SELECT count(*) AS "n!" FROM notes
               WHERE id = $1
                 AND search_vector @@ to_tsquery('simple', 'zebra & plain')
                 AND NOT search_vector @@ to_tsquery('simple', 'markdown')"#,
            note.id.0
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            matched, 1,
            "FTS indexes title + search_text, not raw Markdown"
        );
    }
}
