//! PostgreSQL repository for Second Brain knowledge base & notes.
//!
//! Provides compile-checked `sqlx::query!` / `query_as!` access to `notes`
//! and `knowledge_embeddings` tables, including HNSW vector hybrid search.

use pgvector::Vector;
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::{Note, NoteFormat, NoteId};
use crate::domain::knowledge::search::{KnowledgeItemType, SearchResultItem};

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct NoteRow {
    pub id: i64,
    pub owner_id: uuid::Uuid,
    pub title: String,
    pub content: String,
    pub format: String,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub is_archived: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<NoteRow> for Note {
    fn from(row: NoteRow) -> Self {
        Self {
            id: NoteId(row.id),
            owner_id: AccountId::from_uuid(row.owner_id),
            title: row.title,
            content: row.content,
            format: row.format.parse().unwrap_or(NoteFormat::Markdown),
            source_channel: row.source_channel,
            source_ref: row.source_ref,
            tags: row.tags,
            is_archived: row.is_archived,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub async fn insert_note(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    title: &str,
    content: &str,
    format: &str,
    source_channel: &str,
    source_ref: Option<&str>,
    tags: &[String],
) -> Result<Note, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        INSERT INTO notes (
            owner_id, title, content, format, source_channel, source_ref, tags
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        "#,
        owner.as_uuid(),
        title,
        content,
        format,
        source_channel,
        source_ref,
        tags
    )
    .fetch_one(executor)
    .await?;

    Ok(row.into())
}

pub async fn update_note(
    executor: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
    title: &str,
    content: &str,
    format: &str,
    tags: &[String],
) -> Result<Option<Note>, sqlx::Error> {
    let row = sqlx::query_as!(
        NoteRow,
        r#"
        UPDATE notes
        SET
            title = $3,
            content = $4,
            format = $5,
            tags = $6,
            updated_at = NOW()
        WHERE id = $1 AND owner_id = $2
        RETURNING
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        "#,
        id,
        owner.as_uuid(),
        title,
        content,
        format,
        tags
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(Into::into))
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
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(executor)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list_notes(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    tag_filter: Option<&str>,
    include_archived: bool,
    limit: i64,
) -> Result<Vec<Note>, sqlx::Error> {
    let rows = sqlx::query_as!(
        NoteRow,
        r#"
        SELECT
            id, owner_id, title, content, format, source_channel, source_ref,
            tags, is_archived, created_at, updated_at
        FROM notes
        WHERE owner_id = $1
          AND ($2::TEXT IS NULL OR $2 = ANY(tags))
          AND ($3::BOOLEAN OR is_archived = FALSE)
        ORDER BY updated_at DESC
        LIMIT $4
        "#,
        owner.as_uuid(),
        tag_filter,
        include_archived,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
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

pub async fn upsert_embedding_chunk(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: &str,
    item_id: i64,
    chunk_index: i32,
    content_snippet: &str,
    embedding: &[f32],
) -> Result<(), sqlx::Error> {
    let vec = Vector::from(embedding.to_vec());
    sqlx::query!(
        r#"
        INSERT INTO knowledge_embeddings (
            owner_id, item_type, item_id, chunk_index, content_snippet, embedding, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, NOW())
        ON CONFLICT (owner_id, item_type, item_id, chunk_index)
        DO UPDATE SET
            content_snippet = EXCLUDED.content_snippet,
            embedding = EXCLUDED.embedding,
            updated_at = NOW()
        "#,
        owner.as_uuid(),
        item_type,
        item_id,
        chunk_index,
        content_snippet,
        vec as Vector
    )
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn delete_embeddings_for_item(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: &str,
    item_id: i64,
) -> Result<u64, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        DELETE FROM knowledge_embeddings
        WHERE owner_id = $1 AND item_type = $2 AND item_id = $3
        "#,
        owner.as_uuid(),
        item_type,
        item_id
    )
    .execute(executor)
    .await?;

    Ok(res.rows_affected())
}

#[derive(Debug, sqlx::FromRow)]
struct SearchResultRow {
    item_type: String,
    item_id: i64,
    content_snippet: String,
    title: Option<String>,
    source_ref: Option<String>,
    combined_score: Option<f32>,
}

pub async fn search_hybrid(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    query_embedding: &[f32],
    query_text: &str,
    filter_types: Option<&[String]>,
    limit: i64,
) -> Result<Vec<SearchResultItem>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    let rows = sqlx::query_as!(
        SearchResultRow,
        r#"
        SELECT
            ke.item_type,
            ke.item_id,
            ke.content_snippet,
            COALESCE(n.title, b.title, ke.item_type) AS title,
            COALESCE(n.source_ref, b.url) AS source_ref,
            (
                0.7 * (1.0 - (ke.embedding <=> $2) / 2.0) +
                0.3 * LEAST(1.0::FLOAT8, ts_rank_cd(to_tsvector('english', ke.content_snippet), plainto_tsquery('english', $3)))
            )::FLOAT4 AS combined_score
        FROM knowledge_embeddings ke
        LEFT JOIN notes n
            ON ke.item_type = 'note' AND ke.item_id = n.id AND ke.owner_id = n.owner_id
        LEFT JOIN bookmarks b
            ON ke.item_type = 'bookmark' AND ke.item_id = b.id AND ke.owner_id = b.owner_id
        WHERE ke.owner_id = $1
          AND ($4::TEXT[] IS NULL OR ke.item_type = ANY($4))
        ORDER BY combined_score DESC
        LIMIT $5
        "#,
        owner.as_uuid(),
        vec as Vector,
        query_text,
        filter_types,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| SearchResultItem {
            item_type: KnowledgeItemType::from_str_opt(&r.item_type).unwrap_or(KnowledgeItemType::Note),
            item_id: r.item_id,
            title: r.title.unwrap_or_else(|| r.item_type.clone()),
            content_snippet: r.content_snippet,
            similarity_score: r.combined_score.unwrap_or(0.0),
            source_ref: r.source_ref,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};

    #[tokio::test]
    async fn test_note_crud_and_isolation() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        let bob = AccountId::generate();
        seed_account(&pool, &alice).await;
        seed_account(&pool, &bob).await;

        let note = insert_note(
            &pool,
            &alice,
            "Deep Work",
            "Focus without distraction",
            "markdown",
            "web",
            None,
            &["productivity".to_string(), "focus".to_string()],
        )
        .await
        .expect("insert note");

        assert_eq!(note.title, "Deep Work");
        assert_eq!(note.tags, vec!["productivity", "focus"]);
        assert_eq!(note.owner_id, alice);

        // Bob cannot read Alice's note
        let bob_read = get_note_by_id(&pool, note.id.0, &bob)
            .await
            .expect("query note");
        assert!(bob_read.is_none());

        // Alice reads her note
        let alice_read = get_note_by_id(&pool, note.id.0, &alice)
            .await
            .expect("query note")
            .expect("note found");
        assert_eq!(alice_read.id, note.id);

        // Update note
        let updated = update_note(
            &pool,
            note.id.0,
            &alice,
            "Deep Work 2.0",
            "Updated focus tactics",
            "markdown",
            &["focus".to_string()],
        )
        .await
        .expect("update note")
        .expect("note returned");
        assert_eq!(updated.title, "Deep Work 2.0");
        assert_eq!(updated.tags, vec!["focus"]);

        // List notes with tag filter
        let list_filtered = list_notes(&pool, &alice, Some("focus"), false, 10)
            .await
            .expect("list notes");
        assert_eq!(list_filtered.len(), 1);

        let list_unmatched = list_notes(&pool, &alice, Some("nonexistent"), false, 10)
            .await
            .expect("list notes");
        assert_eq!(list_unmatched.len(), 0);

        // Archive note
        let archived = archive_note(&pool, note.id.0, &alice, true)
            .await
            .expect("archive note");
        assert!(archived);

        let list_active = list_notes(&pool, &alice, None, false, 10)
            .await
            .expect("list active");
        assert_eq!(list_active.len(), 0);

        let list_all = list_notes(&pool, &alice, None, true, 10)
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
    async fn test_knowledge_embeddings_and_hybrid_search() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let note = insert_note(
            &pool,
            &alice,
            "Rust Async Guide",
            "Tokio runtimes and cooperative scheduling in Rust",
            "markdown",
            "matrix",
            Some("!event123"),
            &["rust".to_string(), "async".to_string()],
        )
        .await
        .expect("insert note");

        // Upsert dummy 384-dimensional embedding chunk
        let mut embedding = vec![0.0f32; 384];
        embedding[0] = 1.0;
        upsert_embedding_chunk(
            &pool,
            &alice,
            "note",
            note.id.0,
            0,
            "Tokio runtimes and cooperative scheduling in Rust",
            &embedding,
        )
        .await
        .expect("upsert embedding");

        // Hybrid search with matching embedding and matching text
        let results = search_hybrid(
            &pool,
            &alice,
            &embedding,
            "Tokio cooperative",
            Some(&["note".to_string()]),
            5,
        )
        .await
        .expect("hybrid search");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].item_id, note.id.0);
        assert_eq!(results[0].title, "Rust Async Guide");
        assert_eq!(results[0].item_type, KnowledgeItemType::Note);
        assert!(results[0].similarity_score > 0.6, "expected > 0.6, got {}", results[0].similarity_score);

        // Clean up embeddings
        let affected = delete_embeddings_for_item(&pool, &alice, "note", note.id.0)
            .await
            .expect("delete embeddings");
        assert_eq!(affected, 1);
    }
}
