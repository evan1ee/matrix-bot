//! Retrieval queries behind knowledge search: a lexical list, a vector list and
//! hydration of the fused result keys. Fusion itself is a pure domain function
//! (`domain::knowledge::rrf`); nothing here ranks across lists.

use std::collections::HashMap;

use pgvector::Vector;
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;
use crate::domain::knowledge::search::{ItemKey, KnowledgeItemType};

/// Nearest chunks fetched before keeping the best chunk per item. Several
/// chunks of one note can be near the query; this bounds the work while leaving
/// room for at least the requested number of distinct items.
const NEAREST_CHUNKS: i64 = 200;

/// A lexical match on a non-note item, carrying the chunk that matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkHit {
    pub key: ItemKey,
    pub snippet: String,
}

/// The best-matching chunk of one item for a query vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VectorHit {
    pub key: ItemKey,
    pub snippet: String,
}

/// Display fields of a result, looked up from the item's own table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hydrated {
    pub title: String,
    pub source_ref: Option<String>,
    /// A note's stored excerpt; `None` for the other item types.
    pub excerpt: Option<String>,
}

fn key(item_type: &str, item_id: i64) -> Option<ItemKey> {
    KnowledgeItemType::from_str_opt(item_type).map(|t| ItemKey::new(t, item_id))
}

/// Notes whose full-text vector matches `query`, best first. The vector covers
/// the title (weight A) and the Markdown-stripped text (weight B), so it works
/// before a note has been embedded.
pub async fn search_notes_fts(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    query: &str,
    include_archived: bool,
    limit: i64,
) -> Result<Vec<ItemKey>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT n.id
        FROM notes n, websearch_to_tsquery('simple', $2) q
        WHERE n.owner_id = $1
          AND n.search_vector @@ q
          AND ($3 OR NOT n.is_archived)
        ORDER BY ts_rank_cd(n.search_vector, q) DESC, n.id DESC
        LIMIT $4
        "#,
        owner.as_uuid(),
        query,
        include_archived,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ItemKey::new(KnowledgeItemType::Note, r.id))
        .collect())
}

/// Escapes `%`, `_` and `\` so `query` matches literally inside an `ILIKE`.
fn like_pattern(query: &str) -> String {
    let mut pattern = String::with_capacity(query.len() + 2);
    pattern.push('%');
    for c in query.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('%');
    pattern
}

/// Notes containing `query` as a substring, or a word similar to it, best
/// first (substring matches before fuzzy ones).
///
/// The substring match is what finds Chinese, Japanese and Korean text: the
/// `simple` configuration keeps a run of those characters as one token, and
/// trigram similarity cannot match the middle of such a run because trigrams
/// are padded at word edges. The `pg_trgm` GIN index accelerates the `ILIKE`
/// for queries of three or more characters. `word_similarity` adds tolerance
/// for typos and partial words in other scripts.
pub async fn search_notes_trigram(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    query: &str,
    include_archived: bool,
    limit: i64,
) -> Result<Vec<ItemKey>, sqlx::Error> {
    let pattern = like_pattern(query);
    let rows = sqlx::query!(
        r#"
        SELECT n.id
        FROM notes n
        WHERE n.owner_id = $1
          AND ($3 OR NOT n.is_archived)
          AND ((n.title || ' ' || COALESCE(n.search_text, n.content)) ILIKE $5
               OR word_similarity($2, n.title || ' ' || COALESCE(n.search_text, n.content)) > 0.3)
        ORDER BY ((n.title || ' ' || COALESCE(n.search_text, n.content)) ILIKE $5) DESC,
                 word_similarity($2, n.title || ' ' || COALESCE(n.search_text, n.content)) DESC,
                 n.id DESC
        LIMIT $4
        "#,
        owner.as_uuid(),
        query,
        include_archived,
        limit,
        pattern
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| ItemKey::new(KnowledgeItemType::Note, r.id))
        .collect())
}

/// Bookmarks, journal entries and footprints whose indexed chunks match
/// `query`, best first, with the best matching chunk. Notes are excluded: they
/// have their own vector and are searched without needing chunks.
pub async fn search_chunks_fts(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    query: &str,
    types: &[String],
    include_archived: bool,
    limit: i64,
) -> Result<Vec<ChunkHit>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT item_type AS "item_type!", item_id AS "item_id!", content_snippet AS "content_snippet!"
        FROM (
            SELECT DISTINCT ON (ke.item_type, ke.item_id)
                   ke.item_type, ke.item_id, ke.content_snippet,
                   ts_rank_cd(to_tsvector('simple', ke.content_snippet), q) AS rank
            FROM knowledge_embeddings ke
            LEFT JOIN bookmarks b
                ON ke.item_type = 'bookmark' AND b.id = ke.item_id AND b.owner_id = ke.owner_id,
                 websearch_to_tsquery('simple', $2) q
            WHERE ke.owner_id = $1
              AND ke.item_type <> 'note'
              AND ke.item_type = ANY($3)
              AND to_tsvector('simple', ke.content_snippet) @@ q
              AND ($4 OR NOT COALESCE(b.is_archived, FALSE))
            ORDER BY ke.item_type, ke.item_id,
                     ts_rank_cd(to_tsvector('simple', ke.content_snippet), q) DESC
        ) best
        ORDER BY rank DESC, item_id DESC
        LIMIT $5
        "#,
        owner.as_uuid(),
        query,
        types,
        include_archived,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(ChunkHit {
                key: key(&r.item_type, r.item_id)?,
                snippet: r.content_snippet,
            })
        })
        .collect())
}

/// The items nearest to `query_embedding`, closest first, one entry per item
/// with its best chunk. Only chunks produced by `model` take part: vectors from
/// different models are not comparable.
pub async fn search_vector(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    query_embedding: &[f32],
    model: &str,
    types: &[String],
    include_archived: bool,
    limit: i64,
) -> Result<Vec<VectorHit>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    let rows = sqlx::query!(
        r#"
        WITH nearest AS (
            SELECT ke.item_type, ke.item_id, ke.content_snippet,
                   ke.embedding <=> $2 AS distance
            FROM knowledge_embeddings ke
            LEFT JOIN notes n
                ON ke.item_type = 'note' AND n.id = ke.item_id AND n.owner_id = ke.owner_id
            LEFT JOIN bookmarks b
                ON ke.item_type = 'bookmark' AND b.id = ke.item_id AND b.owner_id = ke.owner_id
            WHERE ke.owner_id = $1
              AND ke.embedding_model = $3
              AND ke.item_type = ANY($4)
              AND ($5 OR NOT COALESCE(n.is_archived, b.is_archived, FALSE))
            ORDER BY ke.embedding <=> $2
            LIMIT $6
        ),
        best AS (
            SELECT DISTINCT ON (item_type, item_id)
                   item_type, item_id, content_snippet, distance
            FROM nearest
            ORDER BY item_type, item_id, distance
        )
        SELECT item_type AS "item_type!", item_id AS "item_id!", content_snippet AS "content_snippet!"
        FROM best
        ORDER BY distance, item_id
        LIMIT $7
        "#,
        owner.as_uuid(),
        vec as Vector,
        model,
        types,
        include_archived,
        NEAREST_CHUNKS,
        limit
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(VectorHit {
                key: key(&r.item_type, r.item_id)?,
                snippet: r.content_snippet,
            })
        })
        .collect())
}

/// Title, source reference and excerpt for result keys, scoped to `owner`.
/// A key whose item no longer exists (a stale embedding) is absent from the map.
pub async fn hydrate(
    executor: impl PgExecutor<'_>,
    owner: &AccountId,
    keys: &[ItemKey],
) -> Result<HashMap<ItemKey, Hydrated>, sqlx::Error> {
    let types: Vec<String> = keys
        .iter()
        .map(|k| k.item_type.as_str().to_string())
        .collect();
    let ids: Vec<i64> = keys.iter().map(|k| k.item_id).collect();
    let rows = sqlx::query!(
        r#"
        SELECT
            k.item_type AS "item_type!",
            k.item_id AS "item_id!",
            COALESCE(n.title, b.title, f.title, l.log_date, k.item_type) AS "title!",
            COALESCE(n.source_ref, b.url) AS source_ref,
            CASE WHEN n.id IS NOT NULL THEN COALESCE(n.excerpt, left(n.content, 200)) END AS excerpt,
            (n.id IS NOT NULL OR b.id IS NOT NULL OR f.id IS NOT NULL OR l.id IS NOT NULL) AS "found!"
        FROM unnest($2::TEXT[], $3::BIGINT[]) AS k(item_type, item_id)
        LEFT JOIN notes n
            ON k.item_type = 'note' AND n.id = k.item_id AND n.owner_id = $1
        LEFT JOIN bookmarks b
            ON k.item_type = 'bookmark' AND b.id = k.item_id AND b.owner_id = $1
        LEFT JOIN footprints f
            ON k.item_type = 'footprint' AND f.id = k.item_id AND f.owner_id = $1
        LEFT JOIN life_day_logs l
            ON k.item_type = 'journal' AND l.id = k.item_id AND l.owner_id = $1
        "#,
        owner.as_uuid(),
        &types,
        &ids
    )
    .fetch_all(executor)
    .await?;

    Ok(rows
        .into_iter()
        .filter(|r| r.found)
        .filter_map(|r| {
            Some((
                key(&r.item_type, r.item_id)?,
                Hydrated {
                    title: r.title,
                    source_ref: r.source_ref,
                    excerpt: r.excerpt,
                },
            ))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::db::DbPool;
    use crate::domain::knowledge::search::LEGACY_EMBEDDING_MODEL;
    use crate::infrastructure::postgres::repositories::knowledge::{
        archive_note, insert_legacy_note, insert_test_note, upsert_embedding_chunk,
    };

    const ALL: [&str; 4] = ["note", "bookmark", "journal", "footprint"];

    fn all() -> Vec<String> {
        ALL.iter().map(|s| s.to_string()).collect()
    }

    fn note_key(id: i64) -> ItemKey {
        ItemKey::new(KnowledgeItemType::Note, id)
    }

    async fn setup() -> (DbPool, AccountId) {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;
        (pool, alice)
    }

    fn unit(axis: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; 384];
        v[axis] = 1.0;
        v
    }

    async fn insert_bookmark(pool: &DbPool, owner: &AccountId, title: &str, archived: bool) -> i64 {
        sqlx::query_scalar!(
            r#"INSERT INTO bookmarks (owner_id, url, title, domain, is_archived)
               VALUES ($1, 'https://example.com/a', $2, 'example.com', $3) RETURNING id"#,
            owner.as_uuid(),
            title,
            archived
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn notes_are_found_by_full_text_before_any_embedding_exists() {
        let (pool, alice) = setup().await;
        let hit = insert_test_note(&pool, &alice, "Gardening", "tomatoes need sunlight", &[])
            .await
            .unwrap();
        insert_test_note(&pool, &alice, "Cooking", "boil the pasta", &[])
            .await
            .unwrap();

        let found = search_notes_fts(&pool, &alice, "sunlight", false, 10)
            .await
            .unwrap();
        assert_eq!(found, vec![note_key(hit.id.0)]);
        // The title is searchable too.
        let by_title = search_notes_fts(&pool, &alice, "gardening", false, 10)
            .await
            .unwrap();
        assert_eq!(by_title, vec![note_key(hit.id.0)]);
        assert!(search_notes_fts(&pool, &alice, "absent", false, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn full_text_ranks_a_title_match_above_a_body_match() {
        let (pool, alice) = setup().await;
        let body = insert_test_note(&pool, &alice, "Misc", "all about rust", &[])
            .await
            .unwrap();
        let title = insert_test_note(&pool, &alice, "Rust", "notes", &[])
            .await
            .unwrap();
        let found = search_notes_fts(&pool, &alice, "rust", false, 10)
            .await
            .unwrap();
        assert_eq!(found, vec![note_key(title.id.0), note_key(body.id.0)]);
    }

    #[tokio::test]
    async fn trigram_finds_short_chinese_queries_in_unsplit_text() {
        let (pool, alice) = setup().await;
        let hit = insert_test_note(&pool, &alice, "猫", "猫坐在垫子上", &[])
            .await
            .unwrap();
        insert_test_note(&pool, &alice, "狗", "狗在院子里跑", &[])
            .await
            .unwrap();

        for query in ["垫子", "坐在垫", "猫坐"] {
            let found = search_notes_trigram(&pool, &alice, query, false, 10)
                .await
                .unwrap();
            assert_eq!(found, vec![note_key(hit.id.0)], "query {query}");
        }
        // The word-based full text cannot split the run, which is why trigram exists.
        assert!(search_notes_fts(&pool, &alice, "垫子", false, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("cat"), "%cat%");
        assert_eq!(like_pattern("100%_x\\y"), "%100\\%\\_x\\\\y%");
    }

    #[tokio::test]
    async fn trigram_search_treats_wildcards_literally_and_tolerates_typos() {
        let (pool, alice) = setup().await;
        let discount = insert_test_note(&pool, &alice, "Sale", "a 100% discount", &[])
            .await
            .unwrap();
        let tomato = insert_test_note(&pool, &alice, "Garden", "growing tomatoes", &[])
            .await
            .unwrap();

        // A bare `%` or `_` must not match every note.
        assert_eq!(
            search_notes_trigram(&pool, &alice, "%", false, 10)
                .await
                .unwrap(),
            vec![note_key(discount.id.0)]
        );
        assert!(search_notes_trigram(&pool, &alice, "_", false, 10)
            .await
            .unwrap()
            .is_empty());
        // A misspelling still finds the note by word similarity.
        assert_eq!(
            search_notes_trigram(&pool, &alice, "tomatos", false, 10)
                .await
                .unwrap(),
            vec![note_key(tomato.id.0)]
        );
    }

    #[tokio::test]
    async fn full_text_ignores_markdown_syntax_in_legacy_and_derived_rows() {
        let (pool, alice) = setup().await;
        // A legacy row has no search_text yet; the generated vector falls back
        // to the raw body, so it is still findable.
        let id = insert_legacy_note(&pool, &alice, "Old", "a legacy walrus")
            .await
            .unwrap();
        let found = search_notes_fts(&pool, &alice, "walrus", false, 10)
            .await
            .unwrap();
        assert_eq!(found, vec![note_key(id)]);
    }

    #[tokio::test]
    async fn archived_notes_are_excluded_unless_requested_and_owners_are_isolated() {
        let (pool, alice) = setup().await;
        let bob = AccountId::generate();
        seed_account(&pool, &bob).await;
        let note = insert_test_note(&pool, &alice, "Hidden", "needle in a haystack", &[])
            .await
            .unwrap();
        archive_note(&pool, note.id.0, &alice, true).await.unwrap();

        assert!(search_notes_fts(&pool, &alice, "needle", false, 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            search_notes_fts(&pool, &alice, "needle", true, 10)
                .await
                .unwrap(),
            vec![note_key(note.id.0)]
        );
        assert!(search_notes_fts(&pool, &bob, "needle", true, 10)
            .await
            .unwrap()
            .is_empty());
        assert!(search_notes_trigram(&pool, &bob, "needle", true, 10)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn chunk_full_text_finds_bookmarks_and_skips_notes_and_archived_items() {
        let (pool, alice) = setup().await;
        let live = insert_bookmark(&pool, &alice, "Live", false).await;
        let archived = insert_bookmark(&pool, &alice, "Archived", true).await;
        let note = insert_test_note(&pool, &alice, "N", "kayak routes", &[])
            .await
            .unwrap();
        for (kind, id) in [
            ("bookmark", live),
            ("bookmark", archived),
            ("note", note.id.0),
        ] {
            upsert_embedding_chunk(
                &pool,
                &alice,
                kind,
                id,
                0,
                "kayak routes along the coast",
                &unit(0),
            )
            .await
            .unwrap();
        }

        let hits = search_chunks_fts(&pool, &alice, "kayak", &all(), false, 10)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].key, ItemKey::new(KnowledgeItemType::Bookmark, live));
        assert!(hits[0].snippet.contains("kayak"));

        let with_archived = search_chunks_fts(&pool, &alice, "kayak", &all(), true, 10)
            .await
            .unwrap();
        assert_eq!(with_archived.len(), 2);

        let only_notes = search_chunks_fts(&pool, &alice, "kayak", &["note".to_string()], true, 10)
            .await
            .unwrap();
        assert!(only_notes.is_empty(), "chunk search never returns notes");
    }

    #[tokio::test]
    async fn vector_search_returns_one_entry_per_item_with_its_best_chunk() {
        let (pool, alice) = setup().await;
        let near = insert_test_note(&pool, &alice, "Near", "x", &[])
            .await
            .unwrap();
        let far = insert_test_note(&pool, &alice, "Far", "y", &[])
            .await
            .unwrap();
        // `near` has two chunks, both close; `far` has one distant chunk.
        upsert_embedding_chunk(&pool, &alice, "note", near.id.0, 0, "weaker chunk", &{
            let mut v = unit(0);
            v[1] = 0.5;
            v
        })
        .await
        .unwrap();
        upsert_embedding_chunk(&pool, &alice, "note", near.id.0, 1, "best chunk", &unit(0))
            .await
            .unwrap();
        upsert_embedding_chunk(&pool, &alice, "note", far.id.0, 0, "far chunk", &unit(5))
            .await
            .unwrap();

        let hits = search_vector(
            &pool,
            &alice,
            &unit(0),
            LEGACY_EMBEDDING_MODEL,
            &all(),
            false,
            10,
        )
        .await
        .unwrap();
        assert_eq!(hits.len(), 2, "two items, not three chunks: {hits:?}");
        assert_eq!(hits[0].key, note_key(near.id.0));
        assert_eq!(hits[0].snippet, "best chunk");
        assert_eq!(hits[1].key, note_key(far.id.0));
    }

    #[tokio::test]
    async fn vector_search_ignores_other_models_other_owners_and_archived_items() {
        let (pool, alice) = setup().await;
        let bob = AccountId::generate();
        seed_account(&pool, &bob).await;
        let current = insert_test_note(&pool, &alice, "Current", "a", &[])
            .await
            .unwrap();
        let stale = insert_test_note(&pool, &alice, "Stale", "b", &[])
            .await
            .unwrap();
        let archived = insert_test_note(&pool, &alice, "Arch", "c", &[])
            .await
            .unwrap();
        let theirs = insert_test_note(&pool, &bob, "Theirs", "d", &[])
            .await
            .unwrap();
        for (owner, id) in [
            (&alice, current.id.0),
            (&alice, stale.id.0),
            (&alice, archived.id.0),
            (&bob, theirs.id.0),
        ] {
            upsert_embedding_chunk(&pool, owner, "note", id, 0, "chunk", &unit(0))
                .await
                .unwrap();
        }
        sqlx::query!(
            "UPDATE knowledge_embeddings SET embedding_model = 'older-model' WHERE item_id = $1",
            stale.id.0
        )
        .execute(&pool)
        .await
        .unwrap();
        archive_note(&pool, archived.id.0, &alice, true)
            .await
            .unwrap();

        let hits = search_vector(
            &pool,
            &alice,
            &unit(0),
            LEGACY_EMBEDDING_MODEL,
            &all(),
            false,
            10,
        )
        .await
        .unwrap();
        let keys: Vec<ItemKey> = hits.iter().map(|h| h.key).collect();
        assert_eq!(keys, vec![note_key(current.id.0)]);

        let with_archived = search_vector(
            &pool,
            &alice,
            &unit(0),
            LEGACY_EMBEDDING_MODEL,
            &all(),
            true,
            10,
        )
        .await
        .unwrap();
        assert_eq!(with_archived.len(), 2);
    }

    #[tokio::test]
    async fn hydrate_returns_display_fields_and_drops_foreign_or_missing_items() {
        let (pool, alice) = setup().await;
        let bob = AccountId::generate();
        seed_account(&pool, &bob).await;
        let note = insert_test_note(&pool, &alice, "Title", "the excerpt body", &[])
            .await
            .unwrap();
        let bookmark = insert_bookmark(&pool, &alice, "Saved page", false).await;
        let theirs = insert_test_note(&pool, &bob, "Theirs", "secret", &[])
            .await
            .unwrap();

        let keys = [
            note_key(note.id.0),
            ItemKey::new(KnowledgeItemType::Bookmark, bookmark),
            note_key(theirs.id.0),
            note_key(999_999),
        ];
        let map = hydrate(&pool, &alice, &keys).await.unwrap();
        assert_eq!(map.len(), 2, "{map:?}");
        let n = &map[&keys[0]];
        assert_eq!(
            (n.title.as_str(), n.excerpt.as_deref()),
            ("Title", Some("the excerpt body"))
        );
        let b = &map[&keys[1]];
        assert_eq!(b.title, "Saved page");
        assert_eq!(b.source_ref.as_deref(), Some("https://example.com/a"));
        assert_eq!(b.excerpt, None);
        assert!(hydrate(&pool, &alice, &[]).await.unwrap().is_empty());
    }
}
