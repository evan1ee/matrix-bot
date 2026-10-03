use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct Bookmark {
    pub id: i64,
    pub owner_id: String,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub domain: String,
    pub reader_content: Option<String>,
    pub reading_time_minutes: i64,
    pub is_favorite: bool,
    pub is_archived: bool,
    pub tags: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::domain::bookmarks::Bookmark> for Bookmark {
    fn from(b: crate::domain::bookmarks::Bookmark) -> Self {
        Self {
            id: b.id,
            owner_id: b.owner_id.to_string(),
            url: b.url,
            title: b.title,
            description: b.description,
            author: b.author,
            image_url: b.image_url,
            domain: b.domain,
            reader_content: b.reader_content,
            reading_time_minutes: b.reading_time_minutes,
            is_favorite: b.is_favorite,
            is_archived: b.is_archived,
            tags: b.tags,
            created_at: b.created_at,
            updated_at: b.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateBookmarkInput {
    pub url: String,
    pub title: Option<String>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, Default)]
pub struct UpdateBookmarkInput {
    pub title: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub is_favorite: Option<bool>,
    pub is_archived: Option<bool>,
}

impl From<UpdateBookmarkInput> for crate::domain::bookmarks::UpdateBookmark {
    fn from(input: UpdateBookmarkInput) -> Self {
        Self {
            title: input.title,
            description: input.description,
            tags: input.tags,
            is_favorite: input.is_favorite,
            is_archived: input.is_archived,
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema, Default)]
pub struct BookmarkQuery {
    pub archived: Option<bool>,
    pub favorite: Option<bool>,
    pub tag: Option<String>,
    pub domain: Option<String>,
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl From<BookmarkQuery> for crate::domain::bookmarks::BookmarkQuery {
    fn from(q: BookmarkQuery) -> Self {
        Self {
            archived: q.archived,
            favorite: q.favorite,
            tag: q.tag,
            domain: q.domain,
            search: q.search,
            limit: q.limit,
            offset: q.offset,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DomainCount {
    pub domain: String,
    pub count: i64,
}

impl From<crate::domain::bookmarks::DomainCount> for DomainCount {
    fn from(d: crate::domain::bookmarks::DomainCount) -> Self {
        Self {
            domain: d.domain,
            count: d.count,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct BookmarkStats {
    pub total_count: i64,
    pub unread_count: i64,
    pub archived_count: i64,
    pub favorite_count: i64,
    pub tags: Vec<String>,
    pub top_domains: Vec<DomainCount>,
}

impl From<crate::domain::bookmarks::BookmarkStats> for BookmarkStats {
    fn from(s: crate::domain::bookmarks::BookmarkStats) -> Self {
        Self {
            total_count: s.total_count,
            unread_count: s.unread_count,
            archived_count: s.archived_count,
            favorite_count: s.favorite_count,
            tags: s.tags,
            top_domains: s.top_domains.into_iter().map(Into::into).collect(),
        }
    }
}

#[utoipa::path(
    get,
    tag = "bookmarks",
    path = "/api/v1/bookmarks",
    params(
        ("archived" = Option<bool>, Query, description = "Filter by archived state"),
        ("favorite" = Option<bool>, Query, description = "Filter by favorite state"),
        ("tag" = Option<String>, Query, description = "Filter by tag"),
        ("domain" = Option<String>, Query, description = "Filter by domain"),
        ("search" = Option<String>, Query, description = "Search query"),
        ("limit" = Option<i64>, Query, description = "Page limit"),
        ("offset" = Option<i64>, Query, description = "Page offset")
    ),
    responses((status = 200, body = Vec<Bookmark>)),
    security(("session_cookie" = []))
)]
pub async fn list_bookmarks(
    user: AuthUser,
    State(state): State<ApiState>,
    Query(query): Query<BookmarkQuery>,
) -> Result<Json<Vec<Bookmark>>, ApiError> {
    let domain_query: crate::domain::bookmarks::BookmarkQuery = query.into();
    let items = state
        .services
        .bookmarks
        .list(&user.account_id, &domain_query)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(items.into_iter().map(Into::into).collect()))
}

#[utoipa::path(
    get,
    tag = "bookmarks",
    path = "/api/v1/bookmarks/stats",
    responses((status = 200, body = BookmarkStats)),
    security(("session_cookie" = []))
)]
pub async fn get_stats(
    user: AuthUser,
    State(state): State<ApiState>,
) -> Result<Json<BookmarkStats>, ApiError> {
    let stats = state
        .services
        .bookmarks
        .get_stats(&user.account_id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(stats.into()))
}

#[utoipa::path(
    post,
    tag = "bookmarks",
    path = "/api/v1/bookmarks",
    request_body = CreateBookmarkInput,
    responses((status = 200, body = Bookmark)),
    security(("session_cookie" = []))
)]
pub async fn create_bookmark(
    user: AuthUser,
    State(state): State<ApiState>,
    Json(payload): Json<CreateBookmarkInput>,
) -> Result<Json<Bookmark>, ApiError> {
    let meta = crate::infrastructure::web::fetch_and_extract_metadata(
        Some(&state.http_client),
        &payload.url,
        payload.title.as_deref(),
    )
    .await;

    let bookmark = state
        .services
        .bookmarks
        .create(
            &user.account_id,
            &payload.url,
            payload.title.as_deref(),
            payload.tags.as_deref(),
            meta,
        )
        .await
        .map_err(ApiError::from)?;

    Ok(Json(bookmark.into()))
}

#[utoipa::path(
    get,
    tag = "bookmarks",
    path = "/api/v1/bookmarks/{id}",
    params(("id" = i64, Path, description = "Bookmark ID")),
    responses((status = 200, body = Bookmark)),
    security(("session_cookie" = []))
)]
pub async fn get_bookmark(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Bookmark>, ApiError> {
    let bookmark = state
        .services
        .bookmarks
        .get(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(bookmark.into()))
}

#[utoipa::path(
    patch,
    tag = "bookmarks",
    path = "/api/v1/bookmarks/{id}",
    params(("id" = i64, Path, description = "Bookmark ID")),
    request_body = UpdateBookmarkInput,
    responses((status = 200, body = Bookmark)),
    security(("session_cookie" = []))
)]
pub async fn update_bookmark(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateBookmarkInput>,
) -> Result<Json<Bookmark>, ApiError> {
    let updated = state
        .services
        .bookmarks
        .update(&user.account_id, id, payload.into())
        .await
        .map_err(ApiError::from)?;

    Ok(Json(updated.into()))
}

#[utoipa::path(
    post,
    tag = "bookmarks",
    path = "/api/v1/bookmarks/{id}/toggle-favorite",
    params(("id" = i64, Path, description = "Bookmark ID")),
    responses((status = 200, body = Bookmark)),
    security(("session_cookie" = []))
)]
pub async fn toggle_favorite(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Bookmark>, ApiError> {
    let updated = state
        .services
        .bookmarks
        .toggle_favorite(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(updated.into()))
}

#[utoipa::path(
    post,
    tag = "bookmarks",
    path = "/api/v1/bookmarks/{id}/toggle-archive",
    params(("id" = i64, Path, description = "Bookmark ID")),
    responses((status = 200, body = Bookmark)),
    security(("session_cookie" = []))
)]
pub async fn toggle_archive(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<Json<Bookmark>, ApiError> {
    let updated = state
        .services
        .bookmarks
        .toggle_archived(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(Json(updated.into()))
}

#[utoipa::path(
    delete,
    tag = "bookmarks",
    path = "/api/v1/bookmarks/{id}",
    params(("id" = i64, Path, description = "Bookmark ID")),
    responses((status = 204)),
    security(("session_cookie" = []))
)]
pub async fn delete_bookmark(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .services
        .bookmarks
        .delete(&user.account_id, id)
        .await
        .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup_state() -> (ApiState, crate::domain::accounts::AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = crate::application::AppServices::new(&pool)
            .identity
            .provision_matrix_for_test("@alice:mbm.one")
            .await;
        let state = ApiState::new(
            Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        (state, owner)
    }

    #[tokio::test]
    async fn test_bookmarks_api_crud_flow() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(owner);

        // 1. Create
        let req = CreateBookmarkInput {
            url: "https://news.ycombinator.com/item?id=42".to_string(),
            title: Some("Discussion on Hacker News".to_string()),
            tags: Some(vec!["news".to_string(), "tech".to_string()]),
        };
        let created = create_bookmark(user.clone(), State(state.clone()), Json(req))
            .await
            .unwrap();
        assert_eq!(created.title, "Discussion on Hacker News");
        assert_eq!(created.domain, "news.ycombinator.com");

        // 2. List
        let list = list_bookmarks(
            user.clone(),
            State(state.clone()),
            Query(BookmarkQuery::default()),
        )
        .await
        .unwrap();
        assert_eq!(list.len(), 1);

        // 3. Toggle Fav
        let fav = toggle_favorite(user.clone(), State(state.clone()), Path(created.id))
            .await
            .unwrap();
        assert!(fav.is_favorite);

        // 4. Delete
        let status = delete_bookmark(user.clone(), State(state.clone()), Path(created.id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
}
