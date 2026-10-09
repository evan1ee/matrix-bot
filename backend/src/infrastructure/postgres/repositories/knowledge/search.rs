//! PostgreSQL repository for knowledge vector embeddings and hybrid search.

use pgvector::Vector;
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::search::{KnowledgeItemType, SearchResultItem};

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
            COALESCE(n.title, b.title, f.title, l.log_date, ke.item_type) AS title,
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
        LEFT JOIN footprints f
            ON ke.item_type = 'footprint' AND ke.item_id = f.id AND ke.owner_id = f.owner_id
        LEFT JOIN life_day_logs l
            ON ke.item_type = 'journal' AND ke.item_id = l.id AND ke.owner_id = l.owner_id
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
            item_type: KnowledgeItemType::from_str_opt(&r.item_type)
                .unwrap_or(KnowledgeItemType::Note),
            item_id: r.item_id,
            title: r.title.unwrap_or_else(|| r.item_type.clone()),
            content_snippet: r.content_snippet,
            similarity_score: r.combined_score.unwrap_or(0.0),
            source_ref: r.source_ref,
        })
        .collect())
}

/// Retrieves textual content for a given knowledge item to be chunked and embedded.
pub async fn get_item_content_for_embedding(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: KnowledgeItemType,
    item_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    match item_type {
        KnowledgeItemType::Note => {
            let row = sqlx::query!(
                r#"
                SELECT title, content
                FROM notes
                WHERE id = $1 AND owner_id = $2
                "#,
                item_id,
                owner.as_uuid()
            )
            .fetch_optional(executor)
            .await?;
            Ok(row.map(|r| format!("{}\n\n{}", r.title, r.content)))
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnindexedItem {
    pub owner_id: AccountId,
    pub item_type: KnowledgeItemType,
    pub item_id: i64,
}

/// Scans for unindexed items across notes, bookmarks, journal logs, and footprints.
pub async fn list_unindexed_items(
    executor: impl PgExecutor<'_>,
    limit: i64,
) -> Result<Vec<UnindexedItem>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        WITH unindexed_notes AS (
            SELECT n.owner_id, 'note' AS item_type, n.id
            FROM notes n
            LEFT JOIN knowledge_embeddings ke
                ON ke.owner_id = n.owner_id AND ke.item_type = 'note' AND ke.item_id = n.id
            WHERE ke.item_id IS NULL AND NOT n.is_archived
            LIMIT $1
        ),
        unindexed_bookmarks AS (
            SELECT b.owner_id, 'bookmark' AS item_type, b.id
            FROM bookmarks b
            LEFT JOIN knowledge_embeddings ke
                ON ke.owner_id = b.owner_id AND ke.item_type = 'bookmark' AND ke.item_id = b.id
            WHERE ke.item_id IS NULL AND NOT b.is_archived
            LIMIT $1
        ),
        unindexed_journal AS (
            SELECT l.owner_id, 'journal' AS item_type, l.id
            FROM life_day_logs l
            LEFT JOIN knowledge_embeddings ke
                ON ke.owner_id = l.owner_id AND ke.item_type = 'journal' AND ke.item_id = l.id
            WHERE ke.item_id IS NULL
            LIMIT $1
        ),
        unindexed_footprints AS (
            SELECT f.owner_id, 'footprint' AS item_type, f.id
            FROM footprints f
            LEFT JOIN knowledge_embeddings ke
                ON ke.owner_id = f.owner_id AND ke.item_type = 'footprint' AND ke.item_id = f.id
            WHERE ke.item_id IS NULL
            LIMIT $1
        )
        SELECT owner_id, item_type, id FROM unindexed_notes
        UNION ALL
        SELECT owner_id, item_type, id FROM unindexed_bookmarks
        UNION ALL
        SELECT owner_id, item_type, id FROM unindexed_journal
        UNION ALL
        SELECT owner_id, item_type, id FROM unindexed_footprints
        LIMIT $1
        "#,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            let item_type = KnowledgeItemType::from_str_opt(r.item_type.as_deref()?)?;
            let owner_uuid = r.owner_id?;
            let id = r.id?;
            let owner_id = AccountId::from_uuid(owner_uuid);
            Some(UnindexedItem {
                owner_id,
                item_type,
                item_id: id,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::infrastructure::postgres::repositories::knowledge::notes::insert_test_note;

    #[tokio::test]
    async fn test_knowledge_embeddings_and_hybrid_search() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let note = insert_test_note(
            &pool,
            &alice,
            "Rust Async Guide",
            "Tokio runtimes and cooperative scheduling in Rust",
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
        assert!(
            results[0].similarity_score > 0.6,
            "expected > 0.6, got {}",
            results[0].similarity_score
        );

        // Clean up embeddings
        let affected = delete_embeddings_for_item(&pool, &alice, "note", note.id.0)
            .await
            .expect("delete embeddings");
        assert_eq!(affected, 1);
    }

    #[tokio::test]
    async fn test_get_item_content_and_unindexed_scan() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let note = insert_test_note(
            &pool,
            &alice,
            "Unindexed Note",
            "Content waiting for embedding",
            &[],
        )
        .await
        .expect("insert note");

        // Content fetch
        let content =
            get_item_content_for_embedding(&pool, &alice, KnowledgeItemType::Note, note.id.0)
                .await
                .expect("fetch content");
        assert_eq!(
            content,
            Some("Unindexed Note\n\nContent waiting for embedding".to_string())
        );

        // Scan unindexed
        let unindexed = list_unindexed_items(&pool, 10)
            .await
            .expect("list unindexed");
        assert!(unindexed
            .iter()
            .any(|u| u.item_id == note.id.0 && u.item_type == KnowledgeItemType::Note));
    }
}
