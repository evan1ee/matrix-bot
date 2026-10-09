//! HTTP REST API handlers and DTOs for notes & knowledge search.

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::application::knowledge::SearchQuery;
use crate::domain::knowledge::error::KnowledgeError;
use crate::domain::knowledge::note::{
    ContentHash, ExpectedHash, NewNote, NoteDocument, NotePatch, NoteType,
};
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

/// Response header telling a client whether a search used semantic ranking.
pub const SEARCH_MODE_HEADER: &str = "x-search-mode";

/// A note without its body, as listed. The excerpt is what a list renders.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct NoteSummary {
    pub id: i64,
    pub owner_id: String,
    pub title: String,
    /// One of `note`, `idea`, `fact`, `observation`, `decision`.
    pub note_type: String,
    pub excerpt: String,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    /// SHA-256 of the Markdown body; send it back as `expected_content_hash`
    /// when saving so a stale editor cannot overwrite a newer write.
    pub content_hash: String,
    pub is_archived: bool,
    pub created_at: String,
    pub updated_at: String,
    /// When the embeddings were last built from `content_hash`.
    pub indexed_at: Option<String>,
}

/// A single note: the summary fields plus the Markdown body.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct Note {
    #[serde(flatten)]
    pub summary: NoteSummary,
    pub content: String,
    /// Always `"markdown"`; kept so single-note readers keep working.
    pub format: String,
}

impl From<&crate::domain::knowledge::note::Note> for NoteSummary {
    fn from(n: &crate::domain::knowledge::note::Note) -> Self {
        Self {
            id: n.id.0,
            owner_id: n.owner_id.to_string(),
            title: n.title.clone(),
            note_type: n.note_type.as_str().to_string(),
            excerpt: n.excerpt.clone(),
            source_channel: n.source_channel.clone(),
            source_ref: n.source_ref.clone(),
            tags: n.tags.clone(),
            content_hash: n.content_hash.to_string(),
            is_archived: n.is_archived,
            created_at: n.created_at.to_rfc3339(),
            updated_at: n.updated_at.to_rfc3339(),
            indexed_at: n.indexed_at.map(|at| at.to_rfc3339()),
        }
    }
}

impl From<NoteDocument> for Note {
    fn from(doc: NoteDocument) -> Self {
        Self {
            summary: NoteSummary::from(&doc.note),
            content: doc.markdown,
            format: "markdown".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateNoteInput {
    pub title: String,
    pub content: String,
    /// Accepted and ignored: notes are Markdown.
    pub format: Option<String>,
    /// One of `note` (default), `idea`, `fact`, `observation`, `decision`.
    pub note_type: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct UpdateNoteInput {
    pub title: Option<String>,
    pub content: Option<String>,
    /// Accepted and ignored: notes are Markdown.
    pub format: Option<String>,
    pub note_type: Option<String>,
    pub tags: Option<Vec<String>>,
    pub is_archived: Option<bool>,
    /// The `content_hash` of the note as the editor loaded it. When it no
    /// longer matches the stored body the update is refused with 409.
    pub expected_content_hash: Option<String>,
}

impl UpdateNoteInput {
    fn into_patch(self) -> Result<NotePatch, KnowledgeError> {
        Ok(NotePatch {
            title: self.title,
            markdown: self.content,
            tags: self.tags,
            note_type: self.note_type.as_deref().map(str::parse).transpose()?,
            is_archived: self.is_archived,
            expected: match self.expected_content_hash {
                Some(hash) => ExpectedHash::Exactly(ContentHash::from_hex(&hash)?),
                None => ExpectedHash::Any,
            },
        })
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema, Default)]
pub struct NoteQuery {
    pub tag: Option<String>,
    /// Case-insensitive text to find in the title or body (at most 200 characters).
    pub search: Option<String>,
    pub archived: Option<bool>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KnowledgeSearchRequest {
    pub query: String,
    pub filter_types: Option<Vec<String>>,
    pub limit: Option<i64>,
    /// Include archived notes and bookmarks (default false).
    pub include_archived: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchResultItem {
    pub item_type: String,
    pub item_id: i64,
    pub title: String,
    pub content_snippet: String,
    pub similarity_score: f32,
    pub source_ref: Option<String>,
}

impl From<crate::domain::knowledge::search::SearchResultItem> for SearchResultItem {
    fn from(item: crate::domain::knowledge::search::SearchResultItem) -> Self {
        Self {
            item_type: item.item_type.as_str().to_string(),
            item_id: item.item_id,
            title: item.title,
            content_snippet: item.content_snippet,
            similarity_score: item.similarity_score,
            source_ref: item.source_ref,
        }
    }
}

#[utoipa::path(
    get,
    tag = "knowledge",
    path = "/api/v1/notes",
    params(
        ("tag" = Option<String>, Query, description = "Filter by tag"),
        ("search" = Option<String>, Query, description = "Text the title or body must contain (case-insensitive)"),
        ("archived" = Option<bool>, Query, description = "Filter by archived state"),
        ("limit" = Option<i64>, Query, description = "Max results (1-100)")
    ),
    responses((status = 200, body = Vec<NoteSummary>)),
    security(("session_cookie" = []))
)]
pub async fn list_notes(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(query): Query<NoteQuery>,
) -> Result<Json<Vec<NoteSummary>>, ApiError> {
    let notes = state
        .services
        .notes
        .list(
            &user.account_id,
            query.tag.as_deref(),
            query.search.as_deref(),
            query.archived.unwrap_or(false),
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(notes.iter().map(NoteSummary::from).collect()))
}

#[utoipa::path(
    get,
    tag = "knowledge",
    path = "/api/v1/notes/tags",
    responses((status = 200, body = Vec<String>)),
    security(("session_cookie" = []))
)]
pub async fn list_tags(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<Vec<String>>, ApiError> {
    let tags = state
        .services
        .notes
        .list_tags(&user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(tags))
}

#[utoipa::path(
    get,
    tag = "knowledge",
    path = "/api/v1/notes/{id}",
    params(("id" = i64, Path, description = "Note ID")),
    responses(
        (status = 200, body = Note),
        (status = 404, description = "Note not found")
    ),
    security(("session_cookie" = []))
)]
pub async fn get_note(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Note>, ApiError> {
    let note = state
        .services
        .notes
        .get(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(note.into()))
}

#[utoipa::path(
    post,
    tag = "knowledge",
    path = "/api/v1/notes",
    request_body = CreateNoteInput,
    responses((status = 201, body = Note)),
    security(("session_cookie" = []))
)]
pub async fn create_note(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<CreateNoteInput>,
) -> Result<(StatusCode, Json<Note>), ApiError> {
    // `format` is accepted for compatibility and ignored: notes are Markdown.
    let note_type = match payload.note_type.as_deref() {
        Some(raw) => raw
            .parse::<NoteType>()
            .map_err(crate::application::AppError::from)?,
        None => NoteType::default(),
    };
    let new = NewNote::new(payload.title, payload.content)
        .note_type(note_type)
        .tags(payload.tags.unwrap_or_default());
    let created = state
        .services
        .notes
        .create(&user.account_id, new)
        .await
        .map_err(ApiError::from)?;
    let document = state
        .services
        .notes
        .get(&user.account_id, created.id.0)
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(document.into())))
}

#[utoipa::path(
    put,
    tag = "knowledge",
    path = "/api/v1/notes/{id}",
    params(("id" = i64, Path, description = "Note ID")),
    request_body = UpdateNoteInput,
    responses(
        (status = 200, body = Note),
        (status = 400, description = "Unknown note type or malformed content hash"),
        (status = 404, description = "Note not found"),
        (status = 409, description = "The note changed since it was loaded")
    ),
    security(("session_cookie" = []))
)]
pub async fn update_note(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateNoteInput>,
) -> Result<Json<Note>, ApiError> {
    let patch = payload
        .into_patch()
        .map_err(crate::application::AppError::from)?;
    let notes = &state.services.notes;
    notes
        .update(&user.account_id, id, patch)
        .await
        .map_err(ApiError::from)?;
    let document = notes
        .get(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(document.into()))
}

#[utoipa::path(
    delete,
    tag = "knowledge",
    path = "/api/v1/notes/{id}",
    params(("id" = i64, Path, description = "Note ID")),
    responses((status = 204, description = "Note deleted")),
    security(("session_cookie" = []))
)]
pub async fn delete_note(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .services
        .notes
        .delete_note(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    tag = "knowledge",
    path = "/api/v1/knowledge/search",
    request_body = KnowledgeSearchRequest,
    responses((
        status = 200,
        body = Vec<SearchResultItem>,
        headers(("X-Search-Mode" = String,
            description = "`hybrid`, or `lexical` when the embedder was unavailable and only keyword results are returned"))
    )),
    security(("session_cookie" = []))
)]
pub async fn search_knowledge(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<KnowledgeSearchRequest>,
) -> Result<(HeaderMap, Json<Vec<SearchResultItem>>), ApiError> {
    let query = SearchQuery::new(&payload.query)
        .filter_types(payload.filter_types.as_deref())
        .limit(payload.limit.unwrap_or(5))
        .include_archived(payload.include_archived.unwrap_or(false));
    let outcome = state
        .services
        .knowledge_search
        .search(&user.account_id, query)
        .await
        .map_err(ApiError::from)?;

    let mut headers = HeaderMap::new();
    headers.insert(
        SEARCH_MODE_HEADER,
        HeaderValue::from_static(outcome.mode.as_str()),
    );
    Ok((
        headers,
        Json(outcome.items.into_iter().map(Into::into).collect()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrate::run_migrations_unlocked;
    use crate::db::pool::{seed_account, test_pool};
    use crate::domain::accounts::AccountId;
    use crate::interface::http::auth::AuthUser;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_notes_rest_api_flow() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;

        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;

        let mock_embedder =
            Arc::new(crate::infrastructure::inference::embedding::MockEmbeddingProvider::new(384));
        let services = crate::application::AppServices::new(&pool).with_embedder(mock_embedder);
        let mut state = ApiState::new(
            crate::db::Database { pool: pool.clone() },
            None,
            String::new(),
            reqwest::Client::new(),
        );
        state.services = services;
        let auth_user = AuthUser::for_account(alice);

        // 1. Create note
        let (status, Json(created)) = create_note(
            auth_user.clone(),
            State(state.clone()),
            Json(CreateNoteInput {
                title: "Rust Ownership".to_string(),
                content: "Affine type system and borrow checker".to_string(),
                format: Some("markdown".to_string()),
                note_type: None,
                tags: Some(vec!["rust".to_string(), "systems".to_string()]),
            }),
        )
        .await
        .expect("create note");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created.summary.title, "Rust Ownership");
        assert_eq!(created.summary.tags, vec!["rust", "systems"]);

        // 2. Get note by ID
        let Json(fetched) = get_note(
            auth_user.clone(),
            State(state.clone()),
            Path(created.summary.id),
        )
        .await
        .expect("get note");
        assert_eq!(fetched.summary.id, created.summary.id);

        // 3. List tags
        let Json(tags) = list_tags(auth_user.clone(), State(state.clone()))
            .await
            .expect("list tags");
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"systems".to_string()));

        // 4. Update note
        let Json(updated) = update_note(
            auth_user.clone(),
            State(state.clone()),
            Path(created.summary.id),
            Json(UpdateNoteInput {
                title: Some("Rust Ownership 2.0".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("update note");
        assert_eq!(updated.summary.title, "Rust Ownership 2.0");

        // 5. Delete note
        let status = delete_note(
            auth_user.clone(),
            State(state.clone()),
            Path(created.summary.id),
        )
        .await
        .expect("delete note");
        assert_eq!(status, StatusCode::NO_CONTENT);

        // 6. Verify deleted
        let res = get_note(
            auth_user.clone(),
            State(state.clone()),
            Path(created.summary.id),
        )
        .await;
        assert!(res.is_err());
    }

    fn create_input(title: &str, content: &str, note_type: Option<&str>) -> Json<CreateNoteInput> {
        Json(CreateNoteInput {
            title: title.to_string(),
            content: content.to_string(),
            format: None,
            note_type: note_type.map(str::to_string),
            tags: None,
        })
    }

    fn edit(content: &str, hash: Option<&str>) -> Json<UpdateNoteInput> {
        Json(UpdateNoteInput {
            content: Some(content.to_string()),
            expected_content_hash: hash.map(str::to_string),
            ..Default::default()
        })
    }

    async fn mock_state() -> (ApiState, AuthUser) {
        search_state(Arc::new(
            crate::infrastructure::inference::embedding::MockEmbeddingProvider::new(384),
        ))
        .await
    }

    async fn created_note(state: &ApiState, user: &AuthUser, content: &str) -> Note {
        let (_, Json(note)) = create_note(
            user.clone(),
            State(state.clone()),
            create_input("Plan", content, None),
        )
        .await
        .expect("create");
        note
    }

    #[tokio::test]
    async fn the_list_returns_summaries_without_the_body() {
        let (state, user) = mock_state().await;
        let _created = create_note(
            user.clone(),
            State(state.clone()),
            create_input("Ownership", "# Heading\n\nBorrow **checker** rules", None),
        )
        .await
        .expect("create");

        let Json(items) = list_notes(user, State(state), Query(NoteQuery::default()))
            .await
            .expect("list");
        let value = serde_json::to_value(&items).unwrap();
        let first = &value[0];
        assert!(
            first.get("content").is_none(),
            "list leaked the body: {first}"
        );
        assert!(
            first.get("format").is_none(),
            "list carries format: {first}"
        );
        assert_eq!(first["excerpt"], "Heading Borrow checker rules");
        assert_eq!(first["note_type"], "note");
        assert_eq!(first["content_hash"].as_str().unwrap().len(), 64);
    }

    #[tokio::test]
    async fn the_list_filters_by_title_or_body_text() {
        let (state, user) = mock_state().await;
        for (title, body) in [("Ownership", "borrow checker"), ("Pasta", "tomato sauce")] {
            let _created = create_note(
                user.clone(),
                State(state.clone()),
                create_input(title, body, None),
            )
            .await
            .expect("create");
        }
        let search = |text: &str| {
            Query(NoteQuery {
                search: Some(text.to_string()),
                ..NoteQuery::default()
            })
        };

        let Json(by_body) = list_notes(user.clone(), State(state.clone()), search("TOMATO"))
            .await
            .expect("search by body");
        assert_eq!(by_body.len(), 1);
        assert_eq!(by_body[0].title, "Pasta");

        let Json(by_title) = list_notes(user.clone(), State(state.clone()), search("owner"))
            .await
            .expect("search by title");
        assert_eq!(by_title[0].title, "Ownership");

        let err = list_notes(user, State(state), search(&"a".repeat(201)))
            .await
            .expect_err("oversized search");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_single_note_carries_its_body_and_markdown_format() {
        let (state, user) = mock_state().await;
        let created = created_note(&state, &user, "Borrow **checker**").await;

        let Json(note) = get_note(user, State(state), Path(created.summary.id))
            .await
            .expect("get");
        assert_eq!(note.content, "Borrow **checker**");
        assert_eq!(note.format, "markdown");
        assert_eq!(note.summary.content_hash, created.summary.content_hash);
        let value = serde_json::to_value(&note).unwrap();
        assert!(
            value.get("content_hash").is_some() && value.get("summary").is_none(),
            "the summary fields sit at the top level of a note: {value}"
        );
    }

    #[tokio::test]
    async fn a_stale_content_hash_is_a_409_and_changes_nothing() {
        let (state, user) = mock_state().await;
        let created = created_note(&state, &user, "v1").await;
        let loaded = created.summary.content_hash.clone();

        let Json(second) = update_note(
            user.clone(),
            State(state.clone()),
            Path(created.summary.id),
            edit("v2", Some(&loaded)),
        )
        .await
        .expect("an edit made from the current hash");
        assert_ne!(second.summary.content_hash, loaded);

        let err = update_note(
            user.clone(),
            State(state.clone()),
            Path(created.summary.id),
            edit("v3", Some(&loaded)),
        )
        .await
        .expect_err("an edit made from a stale hash");
        assert_eq!(err.status, StatusCode::CONFLICT);
        assert_eq!(err.message, "note changed since it was loaded");

        let Json(stored) = get_note(user, State(state), Path(created.summary.id))
            .await
            .unwrap();
        assert_eq!(stored.content, "v2");
    }

    #[tokio::test]
    async fn an_update_without_a_hash_is_last_writer_wins() {
        let (state, user) = mock_state().await;
        let created = created_note(&state, &user, "v1").await;
        let Json(updated) = update_note(
            user,
            State(state),
            Path(created.summary.id),
            edit("v2", None),
        )
        .await
        .expect("no expectation, no conflict");
        assert_eq!(updated.content, "v2");
    }

    #[tokio::test]
    async fn a_malformed_content_hash_is_a_400() {
        let (state, user) = mock_state().await;
        let created = created_note(&state, &user, "v1").await;
        let err = update_note(
            user,
            State(state),
            Path(created.summary.id),
            edit("v2", Some("not-a-hash")),
        )
        .await
        .expect_err("malformed hash");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn note_type_round_trips_through_create_and_update() {
        let (state, user) = mock_state().await;
        let (_, Json(created)) = create_note(
            user.clone(),
            State(state.clone()),
            create_input("Cache it", "Use a read-through cache", Some("idea")),
        )
        .await
        .expect("create typed");
        assert_eq!(created.summary.note_type, "idea");

        let Json(updated) = update_note(
            user.clone(),
            State(state.clone()),
            Path(created.summary.id),
            Json(UpdateNoteInput {
                note_type: Some("decision".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect("retype");
        assert_eq!(updated.summary.note_type, "decision");
        assert_eq!(updated.content, "Use a read-through cache");

        let Json(fetched) = get_note(user, State(state), Path(created.summary.id))
            .await
            .unwrap();
        assert_eq!(fetched.summary.note_type, "decision");
    }

    #[tokio::test]
    async fn an_unknown_note_type_is_a_400_on_create_and_update() {
        let (state, user) = mock_state().await;
        let err = create_note(
            user.clone(),
            State(state.clone()),
            create_input("Plan", "v1", Some("memo")),
        )
        .await
        .expect_err("unknown type on create");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let created = created_note(&state, &user, "v1").await;
        let err = update_note(
            user,
            State(state),
            Path(created.summary.id),
            Json(UpdateNoteInput {
                note_type: Some("memo".to_string()),
                ..Default::default()
            }),
        )
        .await
        .expect_err("unknown type on update");
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    struct DownEmbedder;

    #[async_trait::async_trait]
    impl crate::infrastructure::inference::embedding::EmbeddingProvider for DownEmbedder {
        async fn embed(
            &self,
            _texts: &[String],
        ) -> Result<Vec<Vec<f32>>, crate::infrastructure::inference::embedding::EmbeddingError>
        {
            Err(crate::infrastructure::inference::embedding::EmbeddingError::NotConfigured)
        }
    }

    async fn search_state(
        embedder: Arc<dyn crate::infrastructure::inference::embedding::EmbeddingProvider>,
    ) -> (ApiState, AuthUser) {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        let alice = AccountId::generate();
        seed_account(&pool, &alice).await;
        let services = crate::application::AppServices::new(&pool).with_embedder(embedder);
        let mut state = ApiState::new(
            crate::db::Database { pool: pool.clone() },
            None,
            String::new(),
            reqwest::Client::new(),
        );
        state.services = services;
        (state, AuthUser::for_account(alice))
    }

    fn search_request(query: &str) -> Json<KnowledgeSearchRequest> {
        Json(KnowledgeSearchRequest {
            query: query.to_string(),
            filter_types: None,
            limit: None,
            include_archived: None,
        })
    }

    #[tokio::test]
    async fn search_reports_hybrid_mode_in_a_header() {
        let (state, user) = search_state(Arc::new(
            crate::infrastructure::inference::embedding::MockEmbeddingProvider::new(384),
        ))
        .await;
        state
            .services
            .notes
            .create(
                &user.account_id,
                NewNote::new("Lifetimes", "borrow checker lifetimes"),
            )
            .await
            .unwrap();

        let (headers, Json(items)) =
            search_knowledge(user, State(state), search_request("lifetimes"))
                .await
                .expect("search");
        assert_eq!(headers.get(SEARCH_MODE_HEADER).unwrap(), "hybrid");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_type, "note");
        assert_eq!(items[0].title, "Lifetimes");
    }

    #[tokio::test]
    async fn search_survives_a_down_embedder_and_says_so() {
        let (state, user) = search_state(Arc::new(DownEmbedder)).await;
        state
            .services
            .notes
            .create(
                &user.account_id,
                NewNote::new("Lifetimes", "borrow checker lifetimes"),
            )
            .await
            .unwrap();

        let (headers, Json(items)) =
            search_knowledge(user, State(state), search_request("lifetimes"))
                .await
                .expect("an embedder outage is not an error");
        assert_eq!(headers.get(SEARCH_MODE_HEADER).unwrap(), "lexical");
        assert_eq!(items.len(), 1);
    }
}
