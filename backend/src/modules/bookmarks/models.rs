use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DomainCount {
    pub domain: String,
    pub count: i64,
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
