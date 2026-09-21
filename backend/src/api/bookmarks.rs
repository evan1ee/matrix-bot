use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::bookmarks::metadata::fetch_and_extract_metadata;
use crate::modules::bookmarks::models::{
    Bookmark, BookmarkQuery, BookmarkStats, CreateBookmarkInput, UpdateBookmarkInput,
};
use crate::modules::bookmarks::repository::BookmarksRepository;

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
    let items = BookmarksRepository::list(&state.db.pool, &user.account_id, &query)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(items))
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
    let stats = BookmarksRepository::get_stats(&state.db.pool, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(stats))
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
    let url = payload.url.trim();
    if url.is_empty() {
        return Err(ApiError::bad_request("URL cannot be empty"));
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(ApiError::bad_request(
            "URL must begin with http:// or https://",
        ));
    }
    if url.len() > 2048 {
        return Err(ApiError::bad_request(
            "URL exceeds maximum length of 2048 characters",
        ));
    }

    if let Some(ref t) = payload.title {
        if t.len() > 500 {
            return Err(ApiError::bad_request("Title exceeds 500 characters"));
        }
    }

    if let Some(ref tags) = payload.tags {
        if tags.len() > 20 {
            return Err(ApiError::bad_request("Cannot specify more than 20 tags"));
        }
        for tag in tags {
            if tag.len() > 50 {
                return Err(ApiError::bad_request("Tag exceeds 50 characters"));
            }
        }
    }

    let tags_str = payload
        .tags
        .as_ref()
        .map(|tags| {
            tags.iter()
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default();

    let meta =
        fetch_and_extract_metadata(Some(&state.http_client), url, payload.title.as_deref()).await;

    let bookmark = BookmarksRepository::create(
        &state.db.pool,
        &user.account_id,
        url,
        &meta.title,
        meta.description.as_deref(),
        meta.author.as_deref(),
        meta.image_url.as_deref(),
        &meta.domain,
        meta.reader_content.as_deref(),
        meta.reading_time_minutes,
        &tags_str,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::BookmarkCreated(bookmark.clone()));

    Ok(Json(bookmark))
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
    let bookmark = BookmarksRepository::find_by_id(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Bookmark not found"))?;

    Ok(Json(bookmark))
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
    if let Some(ref title) = payload.title {
        if title.len() > 500 {
            return Err(ApiError::bad_request("Title exceeds 500 characters"));
        }
    }
    if let Some(ref desc) = payload.description {
        if desc.len() > 2000 {
            return Err(ApiError::bad_request("Description exceeds 2000 characters"));
        }
    }

    let updated = BookmarksRepository::update(&state.db.pool, id, &user.account_id, &payload)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::BookmarkUpdated(updated.clone()));

    Ok(Json(updated))
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
    let updated = BookmarksRepository::toggle_favorite(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::BookmarkUpdated(updated.clone()));

    Ok(Json(updated))
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
    let updated = BookmarksRepository::toggle_archived(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::BookmarkUpdated(updated.clone()));

    Ok(Json(updated))
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
    let deleted = BookmarksRepository::delete(&state.db.pool, id, &user.account_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !deleted {
        return Err(ApiError::not_found("Bookmark not found"));
    }

    state.broadcaster.send(SystemEvent::BookmarkDeleted {
        id,
        owner_id: user.account_id,
    });

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::events::EventBroadcaster;

    async fn setup_state() -> (ApiState, String) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let owner =
            crate::accounts::AccountRepository::resolve_or_provision(&pool, "@alice:mbm.one")
                .await
                .unwrap();
        let state = ApiState::new(
            Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        (state, owner)
    }

    #[tokio::test]
    async fn test_bookmarks_api_crud_flow() {
        let (state, owner) = setup_state().await;
        let user = AuthUser::for_account(&owner);

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
