//! HTTP REST API handlers and DTOs for notes & knowledge search.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::knowledge::note::NoteFormat;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct Note {
    pub id: i64,
    pub owner_id: String,
    pub title: String,
    pub content: String,
    pub format: String,
    pub source_channel: String,
    pub source_ref: Option<String>,
    pub tags: Vec<String>,
    pub is_archived: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::domain::knowledge::note::Note> for Note {
    fn from(n: crate::domain::knowledge::note::Note) -> Self {
        Self {
            id: n.id.0,
            owner_id: n.owner_id.to_string(),
            title: n.title,
            content: n.content,
            format: n.format.to_string(),
            source_channel: n.source_channel,
            source_ref: n.source_ref,
            tags: n.tags,
            is_archived: n.is_archived,
            created_at: n.created_at.to_rfc3339(),
            updated_at: n.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateNoteInput {
    pub title: String,
    pub content: String,
    pub format: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct UpdateNoteInput {
    pub title: Option<String>,
    pub content: Option<String>,
    pub format: Option<String>,
    pub tags: Option<Vec<String>>,
    pub is_archived: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, ToSchema, Default)]
pub struct NoteQuery {
    pub tag: Option<String>,
    pub archived: Option<bool>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct KnowledgeSearchRequest {
    pub query: String,
    pub filter_types: Option<Vec<String>>,
    pub limit: Option<i64>,
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
        ("archived" = Option<bool>, Query, description = "Filter by archived state"),
        ("limit" = Option<i64>, Query, description = "Max results (1-100)")
    ),
    responses((status = 200, body = Vec<Note>)),
    security(("session_cookie" = []))
)]
pub async fn list_notes(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(query): Query<NoteQuery>,
) -> Result<Json<Vec<Note>>, ApiError> {
    let notes = state
        .services
        .notes
        .list_notes(
            &user.account_id,
            query.tag.as_deref(),
            query.archived.unwrap_or(false),
            query.limit.unwrap_or(50),
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(notes.into_iter().map(Into::into).collect()))
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
        .get_note(&user.account_id, id)
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
    let format = payload
        .format
        .as_deref()
        .and_then(|f| f.parse::<NoteFormat>().ok());

    let note = state
        .services
        .notes
        .create_note(
            &user.account_id,
            &payload.title,
            &payload.content,
            format,
            Some("web"),
            None,
            payload.tags.as_deref(),
        )
        .await
        .map_err(ApiError::from)?;

    Ok((StatusCode::CREATED, Json(note.into())))
}

#[utoipa::path(
    put,
    tag = "knowledge",
    path = "/api/v1/notes/{id}",
    params(("id" = i64, Path, description = "Note ID")),
    request_body = UpdateNoteInput,
    responses((status = 200, body = Note)),
    security(("session_cookie" = []))
)]
pub async fn update_note(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateNoteInput>,
) -> Result<Json<Note>, ApiError> {
    let format = payload
        .format
        .as_deref()
        .and_then(|f| f.parse::<NoteFormat>().ok());

    let existing = state
        .services
        .notes
        .get_note(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    let title = payload.title.as_deref().unwrap_or(&existing.title);
    let content = payload.content.as_deref().unwrap_or(&existing.content);
    let tags = payload.tags.as_deref();

    let note = state
        .services
        .notes
        .update_note(&user.account_id, id, title, content, format, tags)
        .await
        .map_err(ApiError::from)?;

    if let Some(archived) = payload.is_archived {
        let note = state
            .services
            .notes
            .archive_note(&user.account_id, id, archived)
            .await
            .map_err(ApiError::from)?;
        return Ok(Json(note.into()));
    }

    Ok(Json(note.into()))
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
    responses((status = 200, body = Vec<SearchResultItem>)),
    security(("session_cookie" = []))
)]
pub async fn search_knowledge(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<KnowledgeSearchRequest>,
) -> Result<Json<Vec<SearchResultItem>>, ApiError> {
    let limit = payload.limit.unwrap_or(5);
    let items = state
        .services
        .knowledge_search
        .search(
            &user.account_id,
            &payload.query,
            payload.filter_types.as_deref(),
            limit,
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(items.into_iter().map(Into::into).collect()))
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
                tags: Some(vec!["rust".to_string(), "systems".to_string()]),
            }),
        )
        .await
        .expect("create note");
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created.title, "Rust Ownership");
        assert_eq!(created.tags, vec!["rust", "systems"]);

        // 2. Get note by ID
        let Json(fetched) = get_note(auth_user.clone(), State(state.clone()), Path(created.id))
            .await
            .expect("get note");
        assert_eq!(fetched.id, created.id);

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
            Path(created.id),
            Json(UpdateNoteInput {
                title: Some("Rust Ownership 2.0".to_string()),
                content: None,
                format: None,
                tags: None,
                is_archived: None,
            }),
        )
        .await
        .expect("update note");
        assert_eq!(updated.title, "Rust Ownership 2.0");

        // 5. Delete note
        let status = delete_note(auth_user.clone(), State(state.clone()), Path(created.id))
            .await
            .expect("delete note");
        assert_eq!(status, StatusCode::NO_CONTENT);

        // 6. Verify deleted
        let res = get_note(auth_user.clone(), State(state.clone()), Path(created.id)).await;
        assert!(res.is_err());
    }
}
