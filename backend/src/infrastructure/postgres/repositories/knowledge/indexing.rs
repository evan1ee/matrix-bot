//! Queries that decide what needs (re)embedding and record that it was done.
//!
//! A note is indexed when `indexed_content_hash` equals its `content_hash` and
//! every chunk came from the configured model. Rebuilding the embeddings from
//! the notes and their source tables loses nothing, so every query here can be
//! answered from `notes.content` and the other item tables alone.

use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::note::ContentHash;
use crate::domain::knowledge::search::KnowledgeItemType;

/// What embedding a note needs: its title and the text its chunks are built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteIndexInput {
    pub title: String,
    /// Markdown-stripped text (falls back to the raw body for a note the
    /// derived-field backfill has not reached).
    pub text: String,
    pub content_hash: ContentHash,
    /// The hash the stored chunks were built from, if any.
    pub indexed_content_hash: Option<String>,
}

pub async fn get_note_for_indexing(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    id: i64,
) -> Result<Option<NoteIndexInput>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT
            title,
            search_text AS "text",
            content_hash,
            indexed_content_hash
        FROM notes
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(executor)
    .await?;

    row.map(|r| {
        Ok(NoteIndexInput {
            title: r.title,
            text: r.text,
            content_hash: ContentHash::from_hex(&r.content_hash)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            indexed_content_hash: r.indexed_content_hash,
        })
    })
    .transpose()
}

/// Whether an item has chunks and every one of them came from `model`.
pub async fn chunks_match_model(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: KnowledgeItemType,
    item_id: i64,
    model: &str,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT count(*) AS "total!",
               count(*) FILTER (WHERE embedding_model <> $4) AS "other!"
        FROM knowledge_embeddings
        WHERE owner_id = $1 AND item_type = $2 AND item_id = $3
        "#,
        owner.as_uuid(),
        item_type.as_str(),
        item_id,
        model
    )
    .fetch_one(executor)
    .await?;

    Ok(row.total > 0 && row.other == 0)
}

/// Records that a note's chunks were built from `content_hash`, but only if the
/// note still has that hash and `title`: both are part of every chunk. Returns
/// `false` when the note changed meanwhile, in which case the caller must not
/// keep the chunks it just wrote.
pub async fn mark_note_indexed(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    id: i64,
    title: &str,
    content_hash: &ContentHash,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        r#"
        UPDATE notes
        SET indexed_content_hash = $4, indexed_at = NOW()
        WHERE id = $1 AND owner_id = $2 AND title = $3
          AND content_hash = $4
        "#,
        id,
        owner.as_uuid(),
        title,
        content_hash.as_str()
    )
    .execute(executor)
    .await?;

    Ok(res.rows_affected() > 0)
}

/// Retrieves textual content of a bookmark, journal log or footprint to be
/// chunked and embedded.
pub async fn get_item_content_for_embedding(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: KnowledgeItemType,
    item_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    match item_type {
        // Notes are chunked from their derived plain text with the hash
        // bookkeeping in `get_note_for_indexing`; routing one here would
        // embed raw Markdown.
        KnowledgeItemType::Note => Err(sqlx::Error::Protocol(
            "notes are indexed through get_note_for_indexing".to_string(),
        )),
        KnowledgeItemType::Bookmark => {
            let row = sqlx::query!(
                r#"
                SELECT title, description, reader_content
                FROM bookmarks
                WHERE id = $1 AND owner_id = $2
                "#,
                item_id,
                owner.as_uuid()
            )
            .fetch_optional(executor)
            .await?;
            Ok(row.map(|r| {
                let mut parts = vec![r.title];
                if let Some(desc) = r.description {
                    if !desc.trim().is_empty() {
                        parts.push(desc);
                    }
                }
                if let Some(content) = r.reader_content {
                    if !content.trim().is_empty() {
                        parts.push(content);
                    }
                }
                parts.join("\n\n")
            }))
        }
        KnowledgeItemType::Journal => {
            let row = sqlx::query!(
                r#"
                SELECT log_date, mood, activities, note
                FROM life_day_logs
                WHERE id = $1 AND owner_id = $2
                "#,
                item_id,
                owner.as_uuid()
            )
            .fetch_optional(executor)
            .await?;
            Ok(row.map(|r| {
                let mut parts = vec![format!("Journal Log [{}] Mood: {}", r.log_date, r.mood)];
                if let Some(act) = r.activities {
                    if !act.trim().is_empty() {
                        parts.push(format!("Activities: {act}"));
                    }
                }
                if let Some(note) = r.note {
                    if !note.trim().is_empty() {
                        parts.push(note);
                    }
                }
                parts.join("\n\n")
            }))
        }
        KnowledgeItemType::Footprint => {
            let row = sqlx::query!(
                r#"
                SELECT title, city, country, address
                FROM footprints
                WHERE id = $1 AND owner_id = $2
                "#,
                item_id,
                owner.as_uuid()
            )
            .fetch_optional(executor)
            .await?;
            Ok(row.map(|r| {
                let mut loc = Vec::new();
                if let Some(a) = r.address {
                    if !a.is_empty() {
                        loc.push(a);
                    }
                }
                if let Some(c) = r.city {
                    if !c.is_empty() {
                        loc.push(c);
                    }
                }
                if let Some(co) = r.country {
                    if !co.is_empty() {
                        loc.push(co);
                    }
                }
                if loc.is_empty() {
                    r.title
                } else {
                    format!("{}\nLocation: {}", r.title, loc.join(", "))
                }
            }))
        }
    }
}

/// An item that needs embedding, with the version its job is keyed by so a
/// finished job for an earlier version cannot swallow the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnindexedItem {
    pub owner_id: AccountId,
    pub item_type: KnowledgeItemType,
    pub item_id: i64,
    pub version: String,
}

/// Items whose embeddings are missing or out of date for `model`:
/// - a note whose chunks were built from a different body or title (or never),
/// - a bookmark, journal entry or footprint with no chunks,
/// - any item with a chunk produced by a different model.
///
/// Archived notes and bookmarks are left alone until unarchived.
pub async fn list_items_needing_index(
    executor: impl PgExecutor<'_>,
    model: &str,
    limit: i64,
) -> Result<Vec<UnindexedItem>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT owner_id AS "owner_id!", item_type AS "item_type!", item_id AS "item_id!",
               version AS "version!"
        FROM (
            SELECT DISTINCT ON (owner_id, item_type, item_id)
                   owner_id, item_type, item_id, version
            FROM (
                SELECT n.owner_id, 'note'::TEXT AS item_type, n.id AS item_id,
                       $1::TEXT || ':' || (extract(epoch FROM n.updated_at) * 1000000)::BIGINT::TEXT AS version,
                       0 AS src
                FROM notes n
                WHERE NOT n.is_archived
                  AND n.indexed_content_hash IS DISTINCT FROM n.content_hash
                UNION ALL
                SELECT b.owner_id, 'bookmark'::TEXT, b.id, $1::TEXT, 1
                FROM bookmarks b
                LEFT JOIN knowledge_embeddings ke
                    ON ke.owner_id = b.owner_id AND ke.item_type = 'bookmark' AND ke.item_id = b.id
                WHERE ke.item_id IS NULL AND NOT b.is_archived
                UNION ALL
                SELECT l.owner_id, 'journal'::TEXT, l.id, $1::TEXT, 1
                FROM life_day_logs l
                LEFT JOIN knowledge_embeddings ke
                    ON ke.owner_id = l.owner_id AND ke.item_type = 'journal' AND ke.item_id = l.id
                WHERE ke.item_id IS NULL
                UNION ALL
                SELECT f.owner_id, 'footprint'::TEXT, f.id, $1::TEXT, 1
                FROM footprints f
                LEFT JOIN knowledge_embeddings ke
                    ON ke.owner_id = f.owner_id AND ke.item_type = 'footprint' AND ke.item_id = f.id
                WHERE ke.item_id IS NULL
                UNION ALL
                SELECT DISTINCT ke.owner_id, ke.item_type, ke.item_id, $1::TEXT, 2
                FROM knowledge_embeddings ke
                WHERE ke.embedding_model <> $1
            ) needing
            ORDER BY owner_id, item_type, item_id, src
        ) wanted
        LIMIT $2
        "#,
        model,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(UnindexedItem {
                owner_id: AccountId::from_uuid(r.owner_id),
                item_type: KnowledgeItemType::from_str_opt(&r.item_type)?,
                item_id: r.item_id,
                version: r.version,
            })
        })
        .collect())
}
