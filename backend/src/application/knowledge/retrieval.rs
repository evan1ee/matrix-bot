//! Knowledge search: a lexical list and a vector list fused by reciprocal rank.
//!
//! Notes are found by full text the moment they are written, whether or not
//! they have been embedded yet. If the embedder is down or slow the vector list
//! is dropped and the caller is told the result is keyword-only; search never
//! fails because of the embedder.

use std::collections::HashMap;
use std::time::Instant;

use tracing::warn;

use super::search::SearchService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::knowledge::search::{
    contains_cjk, embedding_prefixes, rrf, ItemKey, KnowledgeItemType, SearchMode, SearchOutcome,
    SearchResultItem, EMBEDDING_DIMENSIONS,
};
use crate::infrastructure::postgres::repositories::knowledge::{self as knowledge_repo, VectorHit};

/// Items taken from each list before fusion.
const LIST_LIMIT: i64 = 50;
/// The conventional reciprocal-rank-fusion constant.
const RRF_K: u32 = 60;
/// Trigram matching also runs for Latin queries when full text found fewer
/// notes than this, which catches typos and partial words.
const TRIGRAM_BELOW: usize = 3;
/// Hard cap on how many results a caller can request.
const MAX_RESULTS: i64 = 50;
/// Longest query accepted: it is matched against every note the owner has and
/// sent to the embedder, so it is bounded like every other user string.
pub const MAX_QUERY_CHARS: usize = 500;

/// What to search for and how to scope it.
#[derive(Debug, Clone)]
pub struct SearchQuery {
    text: String,
    filter_types: Option<Vec<String>>,
    limit: i64,
    include_archived: bool,
}

impl SearchQuery {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            filter_types: None,
            limit: 5,
            include_archived: false,
        }
    }

    /// Restricts results to these item types. Unknown names match nothing.
    pub fn filter_types(mut self, types: Option<&[String]>) -> Self {
        self.filter_types = types.map(<[String]>::to_vec);
        self
    }

    pub fn limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }

    /// Archived notes and bookmarks are left out unless this is set.
    pub fn include_archived(mut self, include: bool) -> Self {
        self.include_archived = include;
        self
    }
}

fn resolve_types(filter: Option<&[String]>) -> Vec<String> {
    match filter {
        None => [
            KnowledgeItemType::Note,
            KnowledgeItemType::Bookmark,
            KnowledgeItemType::Journal,
            KnowledgeItemType::Footprint,
        ]
        .iter()
        .map(|t| t.as_str().to_string())
        .collect(),
        Some(names) => {
            let mut types: Vec<String> = names
                .iter()
                .filter_map(|n| KnowledgeItemType::from_str_opt(n))
                .map(|t| t.as_str().to_string())
                .collect();
            types.sort();
            types.dedup();
            types
        }
    }
}

/// Alternates two ranked lists, `a` first, so neither source starves the other
/// when their native scores are not comparable. Duplicates keep their first
/// position.
fn interleave(a: &[ItemKey], b: &[ItemKey]) -> Vec<ItemKey> {
    let mut out: Vec<ItemKey> = Vec::with_capacity(a.len() + b.len());
    for i in 0..a.len().max(b.len()) {
        for key in [a.get(i), b.get(i)].into_iter().flatten() {
            if !out.contains(key) {
                out.push(*key);
            }
        }
    }
    out
}

/// The lexical ranking plus the matched chunk of each non-note hit, which is
/// the only text such a hit has to show.
struct Lexical {
    keys: Vec<ItemKey>,
    snippets: HashMap<ItemKey, String>,
}

impl SearchService {
    /// Searches across notes, bookmarks, journal entries and footprints for
    /// `owner`. Never returns another owner's items.
    pub async fn search(
        &self,
        owner: &AccountId,
        query: SearchQuery,
    ) -> Result<SearchOutcome, AppError> {
        let text = query.text.trim();
        if text.chars().count() > MAX_QUERY_CHARS {
            return Err(AppError::Validation(format!(
                "search query must be at most {MAX_QUERY_CHARS} characters"
            )));
        }
        let types = resolve_types(query.filter_types.as_deref());
        if text.is_empty() || types.is_empty() {
            return Ok(SearchOutcome {
                items: Vec::new(),
                mode: SearchMode::Hybrid,
            });
        }
        let limit = query.limit.clamp(1, MAX_RESULTS);

        let lexical = self
            .lexical(owner, text, &types, query.include_archived)
            .await?;
        let vector = self
            .vector(owner, text, &types, query.include_archived)
            .await?;
        let mode = if vector.is_some() {
            SearchMode::Hybrid
        } else {
            SearchMode::Lexical
        };
        let vector = vector.unwrap_or_default();

        let vector_keys: Vec<ItemKey> = vector.iter().map(|h| h.key).collect();
        let fused = rrf(&lexical.keys, &vector_keys, RRF_K);
        // Look past `limit`: a stale embedding whose item is gone is dropped
        // during hydration and must not shrink the page.
        let candidates: Vec<(ItemKey, f64)> = fused
            .into_iter()
            .take((limit * 2).min(MAX_RESULTS * 2) as usize)
            .collect();
        let keys: Vec<ItemKey> = candidates.iter().map(|(k, _)| *k).collect();
        let hydrated = knowledge_repo::hydrate(&self.pool, owner, &keys).await?;

        let vector_snippets: HashMap<ItemKey, &str> =
            vector.iter().map(|h| (h.key, h.snippet.as_str())).collect();
        // The best a single item can score is one first-place rank per list used.
        let lists_used = if mode == SearchMode::Hybrid { 2.0 } else { 1.0 };
        let best_possible = lists_used / (f64::from(RRF_K) + 1.0);

        let items = candidates
            .into_iter()
            .filter_map(|(key, score)| {
                let shown = hydrated.get(&key)?;
                let snippet = vector_snippets
                    .get(&key)
                    .map(|s| (*s).to_string())
                    .or_else(|| lexical.snippets.get(&key).cloned())
                    .or_else(|| shown.excerpt.clone())
                    .unwrap_or_default();
                Some(SearchResultItem {
                    item_type: key.item_type,
                    item_id: key.item_id,
                    title: shown.title.clone(),
                    content_snippet: snippet,
                    similarity_score: (score / best_possible).clamp(0.0, 1.0) as f32,
                    source_ref: shown.source_ref.clone(),
                })
            })
            .take(limit as usize)
            .collect();

        Ok(SearchOutcome { items, mode })
    }

    async fn lexical(
        &self,
        owner: &AccountId,
        text: &str,
        types: &[String],
        include_archived: bool,
    ) -> Result<Lexical, AppError> {
        let want_notes = types.iter().any(|t| t == "note");
        let other_types: Vec<String> = types.iter().filter(|t| *t != "note").cloned().collect();

        let note_keys = if want_notes {
            knowledge_repo::search_notes_fts(&self.pool, owner, text, include_archived, LIST_LIMIT)
                .await?
        } else {
            Vec::new()
        };
        let chunk_hits = if other_types.is_empty() {
            Vec::new()
        } else {
            knowledge_repo::search_chunks_fts(
                &self.pool,
                owner,
                text,
                &other_types,
                include_archived,
                LIST_LIMIT,
            )
            .await?
        };

        let chunk_keys: Vec<ItemKey> = chunk_hits.iter().map(|h| h.key).collect();
        let mut keys = interleave(&note_keys, &chunk_keys);
        if want_notes && (contains_cjk(text) || note_keys.len() < TRIGRAM_BELOW) {
            let similar = knowledge_repo::search_notes_trigram(
                &self.pool,
                owner,
                text,
                include_archived,
                LIST_LIMIT,
            )
            .await?;
            for key in similar {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys.truncate(LIST_LIMIT as usize);

        let snippets = chunk_hits.into_iter().map(|h| (h.key, h.snippet)).collect();
        Ok(Lexical { keys, snippets })
    }

    /// The vector ranking, or `None` when the embedder could not produce a
    /// query vector in time. A database failure is still an error.
    async fn vector(
        &self,
        owner: &AccountId,
        text: &str,
        types: &[String],
        include_archived: bool,
    ) -> Result<Option<Vec<VectorHit>>, AppError> {
        // No embedder configured is a standing condition, not a failure worth
        // a warning per search.
        let embedder = self.active_embedder();
        if !embedder.is_available() {
            return Ok(None);
        }
        let started = Instant::now();
        // Asymmetric retrieval models embed a query differently from a stored
        // passage; the prefix is for the embedder only.
        let query_input = format!("{}{text}", embedding_prefixes(&self.model).query);
        let embedded =
            tokio::time::timeout(self.embed_timeout, embedder.embed(&[query_input])).await;
        let degraded = |error_kind: &str| {
            warn!(
                operation = "knowledge.search",
                outcome = "degraded",
                error_kind,
                elapsed_ms = started.elapsed().as_millis() as u64,
                "search falls back to keyword results"
            );
        };
        let embedding = match embedded {
            Err(_) => {
                degraded("timeout");
                return Ok(None);
            }
            Ok(Err(_)) => {
                degraded("embedder");
                return Ok(None);
            }
            Ok(Ok(vectors)) => match vectors.into_iter().next() {
                // A vector of another size would make the distance query fail
                // with a database error; treat it as an embedder fault.
                Some(vector) if vector.len() == EMBEDDING_DIMENSIONS => vector,
                Some(_) => {
                    degraded("dimension");
                    return Ok(None);
                }
                None => {
                    degraded("empty_response");
                    return Ok(None);
                }
            },
        };

        let hits = knowledge_repo::search_vector(
            &self.pool,
            owner,
            &embedding,
            &self.model,
            types,
            include_archived,
            LIST_LIMIT,
        )
        .await?;
        Ok(Some(hits))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;

    use super::*;
    use crate::application::knowledge::NoteService;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::db::DbPool;
    use crate::domain::knowledge::note::NewNote;
    use crate::infrastructure::inference::embedding::{
        EmbeddingError, EmbeddingProvider, MockEmbeddingProvider,
    };
    use crate::infrastructure::postgres::repositories::knowledge::upsert_embedding_chunk;

    struct FailingEmbedder;

    #[async_trait]
    impl EmbeddingProvider for FailingEmbedder {
        async fn embed(&self, _texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            Err(EmbeddingError::Unreachable("down".to_string()))
        }
    }

    /// Remembers the texts it was asked to embed.
    struct Recording(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl EmbeddingProvider for Recording {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            self.0.lock().unwrap().extend(texts.iter().cloned());
            Ok(texts.iter().map(|_| vec![0.0; 384]).collect())
        }
    }

    struct SlowEmbedder;

    #[async_trait]
    impl EmbeddingProvider for SlowEmbedder {
        async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(texts.iter().map(|_| vec![0.0; 384]).collect())
        }
    }

    struct Fixture {
        pool: DbPool,
        notes: NoteService,
        search: SearchService,
        alice: AccountId,
    }

    async fn fixture() -> Fixture {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;
        let embedder = Arc::new(MockEmbeddingProvider::new(384));
        Fixture {
            notes: NoteService::new(pool.clone()),
            search: SearchService::new(pool.clone(), embedder),
            pool,
            alice,
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

        /// Indexes `chunk` for a note exactly as the embedder would for `chunk`.
        async fn index(&self, note_id: i64, chunk_index: i32, chunk: &str) {
            let embedding = MockEmbeddingProvider::new(384)
                .embed(&[chunk.to_string()])
                .await
                .unwrap()
                .remove(0);
            upsert_embedding_chunk(
                &self.pool,
                &self.alice,
                "note",
                note_id,
                chunk_index,
                chunk,
                &embedding,
                &self.search.model,
            )
            .await
            .unwrap();
        }

        async fn find(&self, text: &str) -> SearchOutcome {
            self.search
                .search(&self.alice, SearchQuery::new(text).limit(10))
                .await
                .unwrap()
        }
    }

    #[tokio::test]
    async fn a_query_longer_than_the_cap_is_a_validation_error() {
        let f = fixture().await;
        let long = "a".repeat(MAX_QUERY_CHARS + 1);

        let err = f
            .search
            .search(&f.alice, SearchQuery::new(long))
            .await
            .unwrap_err();

        assert!(matches!(err, AppError::Validation(_)), "{err:?}");
        let at_the_cap = "a".repeat(MAX_QUERY_CHARS);
        assert!(f
            .search
            .search(&f.alice, SearchQuery::new(at_the_cap))
            .await
            .is_ok());
    }

    #[tokio::test]
    async fn a_query_vector_of_the_wrong_size_degrades_to_keywords_instead_of_failing() {
        let f = fixture().await;
        f.note("Rust", "walrus notes").await;
        let search = SearchService::new(f.pool.clone(), Arc::new(MockEmbeddingProvider::new(8)));

        let outcome = search
            .search(&f.alice, SearchQuery::new("walrus").limit(10))
            .await
            .expect("a wrong-size vector must not be a 500");

        assert_eq!(outcome.mode, SearchMode::Lexical);
        assert_eq!(outcome.items.len(), 1);
    }

    #[test]
    fn interleave_alternates_and_deduplicates() {
        let n = |id| ItemKey::new(KnowledgeItemType::Note, id);
        let b = |id| ItemKey::new(KnowledgeItemType::Bookmark, id);
        assert_eq!(
            interleave(&[n(1), n(2), n(3)], &[b(9), n(2)]),
            vec![n(1), b(9), n(2), n(3)]
        );
        assert!(interleave(&[], &[]).is_empty());
    }

    #[test]
    fn unknown_type_filters_resolve_to_nothing() {
        assert_eq!(resolve_types(None).len(), 4);
        assert_eq!(
            resolve_types(Some(&["note".to_string(), "NOTE".to_string()])),
            vec!["note".to_string()]
        );
        assert!(resolve_types(Some(&["spreadsheet".to_string()])).is_empty());
    }

    #[tokio::test]
    async fn the_query_is_embedded_with_the_models_query_prefix() {
        let f = fixture().await;
        f.note("Rust", "ownership").await;

        let recorder = Arc::new(Recording(std::sync::Mutex::new(Vec::new())));
        let e5 = f
            .search
            .with_embedder(recorder.clone())
            .with_model("multilingual-e5-small");
        e5.search(&f.alice, SearchQuery::new("  ownership  "))
            .await
            .unwrap();
        // A model without prefixes embeds the bare query.
        f.search
            .with_embedder(recorder.clone())
            .with_model("bge-small-en-v1.5")
            .search(&f.alice, SearchQuery::new("ownership"))
            .await
            .unwrap();

        assert_eq!(
            *recorder.0.lock().unwrap(),
            vec!["query: ownership".to_string(), "ownership".to_string()]
        );
    }

    #[tokio::test]
    async fn a_new_note_is_found_before_it_has_been_embedded() {
        let f = fixture().await;
        let id = f
            .note("Neural Networks", "Backpropagation and gradient descent")
            .await;

        let outcome = f.find("backpropagation").await;
        assert_eq!(outcome.mode, SearchMode::Hybrid);
        assert_eq!(outcome.items.len(), 1);
        let item = &outcome.items[0];
        assert_eq!(
            (item.item_type, item.item_id),
            (KnowledgeItemType::Note, id)
        );
        assert_eq!(item.title, "Neural Networks");
        assert_eq!(item.content_snippet, "Backpropagation and gradient descent");
        assert!((0.0..=1.0).contains(&item.similarity_score));
    }

    #[tokio::test]
    async fn the_embedder_failing_degrades_to_keyword_results() {
        let f = fixture().await;
        let id = f.note("Rust", "ownership and borrowing").await;
        let service = f.search.with_embedder(Arc::new(FailingEmbedder));

        let outcome = service
            .search(&f.alice, SearchQuery::new("ownership"))
            .await
            .expect("an embedder outage must not fail search");
        assert_eq!(outcome.mode, SearchMode::Lexical);
        assert_eq!(outcome.items.len(), 1);
        assert_eq!(outcome.items[0].item_id, id);
    }

    #[tokio::test]
    async fn a_slow_embedder_degrades_instead_of_stalling_search() {
        let f = fixture().await;
        f.note("Rust", "ownership and borrowing").await;
        let service = f
            .search
            .with_embedder(Arc::new(SlowEmbedder))
            .with_embed_timeout(Duration::from_millis(50));

        let started = Instant::now();
        let outcome = service
            .search(&f.alice, SearchQuery::new("ownership"))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(outcome.mode, SearchMode::Lexical);
        assert_eq!(outcome.items.len(), 1);
    }

    #[tokio::test]
    async fn a_short_chinese_query_finds_a_note() {
        let f = fixture().await;
        let id = f.note("猫", "猫坐在垫子上").await;
        f.note("狗", "狗在院子里跑").await;

        let outcome = f.find("垫子").await;
        let ids: Vec<i64> = outcome.items.iter().map(|i| i.item_id).collect();
        assert_eq!(ids, vec![id]);
    }

    #[tokio::test]
    async fn an_item_takes_one_slot_however_many_chunks_match() {
        let f = fixture().await;
        let id = f.note("Tokio", "tokio runtime scheduling").await;
        for (i, chunk) in ["tokio runtime scheduling", "tokio tasks", "tokio io driver"]
            .iter()
            .enumerate()
        {
            f.index(id, i as i32, chunk).await;
        }

        let outcome = f.find("tokio runtime scheduling").await;
        assert_eq!(outcome.items.len(), 1, "{:?}", outcome.items);
        assert_eq!(
            outcome.items[0].content_snippet, "tokio runtime scheduling",
            "the snippet is the best chunk"
        );
    }

    #[tokio::test]
    async fn an_item_found_by_both_lists_outranks_one_found_by_one() {
        let f = fixture().await;
        let both = f.note("Gradient", "gradient descent explained").await;
        let lexical_only = f.note("Gradient notes", "more about gradient").await;
        // Only `both` has a chunk the mock embedder places on the query vector.
        f.index(both, 0, "gradient").await;

        let outcome = f.find("gradient").await;
        let ids: Vec<i64> = outcome.items.iter().map(|i| i.item_id).collect();
        assert_eq!(ids.first(), Some(&both), "{ids:?}");
        assert!(ids.contains(&lexical_only));
        assert!(outcome.items[0].similarity_score > outcome.items[1].similarity_score);
    }

    #[tokio::test]
    async fn archived_notes_need_include_archived() {
        let f = fixture().await;
        let id = f.note("Old plans", "obsolete roadmap").await;
        f.notes.archive_note(&f.alice, id, true).await.unwrap();

        assert!(f.find("roadmap").await.items.is_empty());
        let outcome = f
            .search
            .search(&f.alice, SearchQuery::new("roadmap").include_archived(true))
            .await
            .unwrap();
        assert_eq!(outcome.items.len(), 1);
    }

    #[tokio::test]
    async fn another_owners_notes_are_never_returned() {
        let f = fixture().await;
        f.note("Private", "classified blueprint").await;
        let bob = AccountId::generate();
        seed_account(&f.pool, &bob).await;

        let outcome = f
            .search
            .search(&bob, SearchQuery::new("blueprint"))
            .await
            .unwrap();
        assert!(outcome.items.is_empty());
    }

    #[tokio::test]
    async fn type_filters_and_blank_queries_are_honoured() {
        let f = fixture().await;
        f.note("Rust", "ownership").await;

        let only_bookmarks = f
            .search
            .search(
                &f.alice,
                SearchQuery::new("ownership").filter_types(Some(&["bookmark".to_string()])),
            )
            .await
            .unwrap();
        assert!(only_bookmarks.items.is_empty());
        let unknown = f
            .search
            .search(
                &f.alice,
                SearchQuery::new("ownership").filter_types(Some(&["spam".to_string()])),
            )
            .await
            .unwrap();
        assert!(unknown.items.is_empty());
        assert!(f.find("   ").await.items.is_empty());
    }

    #[tokio::test]
    async fn a_bookmark_is_found_through_its_indexed_chunk() {
        let f = fixture().await;
        let bookmark = sqlx::query_scalar!(
            r#"INSERT INTO bookmarks (owner_id, url, title, domain)
               VALUES ($1, 'https://example.com/k', 'Kayak guide', 'example.com') RETURNING id"#,
            f.alice.as_uuid()
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        let embedding = vec![0.0f32; 384];
        upsert_embedding_chunk(
            &f.pool,
            &f.alice,
            "bookmark",
            bookmark,
            0,
            "paddling routes along the coast",
            &embedding,
            &f.search.model,
        )
        .await
        .unwrap();

        let service = f.search.with_embedder(Arc::new(FailingEmbedder));
        let outcome = service
            .search(&f.alice, SearchQuery::new("paddling"))
            .await
            .unwrap();
        assert_eq!(outcome.items.len(), 1);
        let item = &outcome.items[0];
        assert_eq!(item.item_type, KnowledgeItemType::Bookmark);
        assert_eq!(item.title, "Kayak guide");
        assert_eq!(item.source_ref.as_deref(), Some("https://example.com/k"));
        assert_eq!(item.content_snippet, "paddling routes along the coast");
    }

    #[tokio::test]
    async fn a_stale_embedding_for_a_deleted_note_is_dropped() {
        let f = fixture().await;
        let live = f.note("Live", "shared phrase here").await;
        f.index(live, 0, "shared phrase here").await;
        // An embedding whose note was deleted without its chunks being purged.
        f.index(987_654, 0, "shared phrase here").await;

        let outcome = f.find("shared phrase here").await;
        let ids: Vec<i64> = outcome.items.iter().map(|i| i.item_id).collect();
        assert_eq!(ids, vec![live]);
    }
}
