//! PostgreSQL repository for knowledge embedding chunks.

use pgvector::Vector;
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

/// Stores one chunk, replacing the chunk at the same index. `model` records
/// which embedding model produced the vector, so chunks from a model that is no
/// longer configured can be found and rebuilt.
#[allow(clippy::too_many_arguments)]
pub async fn upsert_embedding_chunk(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: &str,
    item_id: i64,
    chunk_index: i32,
    content_snippet: &str,
    embedding: &[f32],
    model: &str,
) -> Result<(), sqlx::Error> {
    let vec = Vector::from(embedding.to_vec());
    sqlx::query!(
        r#"
        INSERT INTO knowledge_embeddings (
            owner_id, item_type, item_id, chunk_index, content_snippet, embedding,
            embedding_model, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
        ON CONFLICT (owner_id, item_type, item_id, chunk_index)
        DO UPDATE SET
            content_snippet = EXCLUDED.content_snippet,
            embedding = EXCLUDED.embedding,
            embedding_model = EXCLUDED.embedding_model,
            updated_at = NOW()
        "#,
        owner.as_uuid(),
        item_type,
        item_id,
        chunk_index,
        content_snippet,
        vec as Vector,
        model
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

/// A stored chunk as tests compare it.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkRecord {
    pub chunk_index: i32,
    pub snippet: String,
    pub model: String,
    pub embedding: Vec<f32>,
}

/// Every chunk of an item in chunk order.
#[cfg(test)]
pub async fn list_chunks(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    item_type: &str,
    item_id: i64,
) -> Result<Vec<ChunkRecord>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT chunk_index, content_snippet, embedding_model, embedding AS "embedding!: Vector"
        FROM knowledge_embeddings
        WHERE owner_id = $1 AND item_type = $2 AND item_id = $3
        ORDER BY chunk_index
        "#,
        owner.as_uuid(),
        item_type,
        item_id
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ChunkRecord {
            chunk_index: r.chunk_index,
            snippet: r.content_snippet,
            model: r.embedding_model,
            embedding: r.embedding.to_vec(),
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
    async fn embedding_chunks_upsert_and_delete_per_item() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;
        let note = insert_test_note(&pool, &alice, "Rust Async Guide", "Tokio runtimes", &[])
            .await
            .expect("insert note");

        let mut embedding = vec![0.0f32; 384];
        embedding[0] = 1.0;
        for chunk in 0..2 {
            upsert_embedding_chunk(
                &pool, &alice, "note", note.id.0, chunk, "chunk", &embedding, "model-a",
            )
            .await
            .expect("upsert embedding");
        }
        // Upserting the same chunk again replaces it, and re-tags its model.
        upsert_embedding_chunk(
            &pool, &alice, "note", note.id.0, 0, "again", &embedding, "model-b",
        )
        .await
        .expect("upsert embedding");

        let chunks = list_chunks(&pool, &alice, "note", note.id.0).await.unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(
            (chunks[0].snippet.as_str(), chunks[0].model.as_str()),
            ("again", "model-b")
        );
        assert_eq!(chunks[1].model, "model-a");
        assert_eq!(chunks[0].embedding.len(), 384);

        let affected = delete_embeddings_for_item(&pool, &alice, "note", note.id.0)
            .await
            .expect("delete embeddings");
        assert_eq!(affected, 2);
    }
}
