//! PostgreSQL repository for bookmarks.
//!
//! Provides compile-checked `sqlx::query!` / `query_as!` / `query_scalar!`
//! access to the `bookmarks` table.

use sqlx::{PgExecutor, PgPool};

use crate::domain::accounts::AccountId;
use crate::domain::bookmarks::{
    Bookmark, BookmarkQuery, BookmarkStats, CreateBookmark, DomainCount,
};

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct BookmarkRow {
    pub id: i64,
    pub owner_id: uuid::Uuid,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub domain: String,
    pub reader_content: Option<String>,
    pub reading_time_minutes: Option<i64>,
    pub is_favorite: bool,
    pub is_archived: bool,
    pub tags: String,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

impl From<BookmarkRow> for Bookmark {
    fn from(row: BookmarkRow) -> Self {
        Self {
            id: row.id,
            owner_id: AccountId::from_uuid(row.owner_id),
            url: row.url,
            title: row.title,
            description: row.description,
            author: row.author,
            image_url: row.image_url,
            domain: row.domain,
            reader_content: row.reader_content,
            reading_time_minutes: row.reading_time_minutes.unwrap_or(1),
            is_favorite: row.is_favorite,
            is_archived: row.is_archived,
            tags: row.tags,
            created_at: row.created_at.unwrap_or_default(),
            updated_at: row.updated_at.unwrap_or_default(),
        }
    }
}

pub async fn create(
    conn: impl PgExecutor<'_>,
    owner: &AccountId,
    input: &CreateBookmark,
) -> Result<Bookmark, sqlx::Error> {
    let row = sqlx::query_as!(
        BookmarkRow,
        r#"
        INSERT INTO bookmarks (
            owner_id, url, title, description, author, image_url,
            domain, reader_content, reading_time_minutes, is_favorite,
            is_archived, tags, created_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, FALSE, FALSE, $10, CAST(CURRENT_TIMESTAMP AS TEXT), CAST(CURRENT_TIMESTAMP AS TEXT))
        RETURNING id, owner_id, url, title, description, author, image_url,
                  domain, reader_content, reading_time_minutes, is_favorite,
                  is_archived, tags, created_at, updated_at
        "#,
        owner.as_uuid(),
        input.url,
        input.title,
        input.description,
        input.author,
        input.image_url,
        input.domain,
        input.reader_content,
        input.reading_time_minutes,
        input.tags,
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find_by_id(
    conn: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Bookmark>, sqlx::Error> {
    let row = sqlx::query_as!(
        BookmarkRow,
        r#"
        SELECT id, owner_id, url, title, description, author, image_url,
               domain, reader_content, reading_time_minutes, is_favorite,
               is_archived, tags, created_at, updated_at
        FROM bookmarks
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn list(
    conn: impl PgExecutor<'_>,
    owner: &AccountId,
    query: &BookmarkQuery,
) -> Result<Vec<Bookmark>, sqlx::Error> {
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);

    let domain_filter = query
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tag_clean = query
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tag_pattern = tag_clean.map(|t| format!("%{t}%"));
    let search_clean = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let search_pattern = search_clean.map(|s| format!("%{s}%"));

    let rows = sqlx::query_as!(
        BookmarkRow,
        r#"
        SELECT id, owner_id, url, title, description, author, image_url,
               domain, reader_content, reading_time_minutes, is_favorite,
               is_archived, tags, created_at, updated_at
        FROM bookmarks
        WHERE owner_id = $1
          AND ($2::boolean IS NULL OR is_archived = $2)
          AND ($3::boolean IS NULL OR is_favorite = $3)
          AND ($4::text IS NULL OR domain = $4)
          AND ($5::text IS NULL OR tags LIKE $5 OR tags = $6)
          AND ($7::text IS NULL OR title LIKE $7 OR description LIKE $7 OR url LIKE $7)
        ORDER BY id DESC
        LIMIT $8
        OFFSET $9
        "#,
        owner.as_uuid(),
        query.archived,
        query.favorite,
        domain_filter,
        tag_pattern.as_deref(),
        tag_clean,
        search_pattern.as_deref(),
        limit,
        offset,
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count(
    conn: impl PgExecutor<'_>,
    owner: &AccountId,
    query: &BookmarkQuery,
) -> Result<i64, sqlx::Error> {
    let domain_filter = query
        .domain
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tag_clean = query
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let tag_pattern = tag_clean.map(|t| format!("%{t}%"));
    let search_clean = query
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let search_pattern = search_clean.map(|s| format!("%{s}%"));

    let total = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*) as "count!"
        FROM bookmarks
        WHERE owner_id = $1
          AND ($2::boolean IS NULL OR is_archived = $2)
          AND ($3::boolean IS NULL OR is_favorite = $3)
          AND ($4::text IS NULL OR domain = $4)
          AND ($5::text IS NULL OR tags LIKE $5 OR tags = $6)
          AND ($7::text IS NULL OR title LIKE $7 OR description LIKE $7 OR url LIKE $7)
        "#,
        owner.as_uuid(),
        query.archived,
        query.favorite,
        domain_filter,
        tag_pattern.as_deref(),
        tag_clean,
        search_pattern.as_deref(),
    )
    .fetch_one(conn)
    .await?;

    Ok(total)
}

pub async fn update_fields(
    conn: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
    title: &str,
    description: Option<&str>,
    is_favorite: bool,
    is_archived: bool,
    tags: &str,
) -> Result<Option<Bookmark>, sqlx::Error> {
    let row = sqlx::query_as!(
        BookmarkRow,
        r#"
        UPDATE bookmarks
        SET title = $1, description = $2, is_favorite = $3, is_archived = $4, tags = $5,
            updated_at = CAST(CURRENT_TIMESTAMP AS TEXT)
        WHERE id = $6 AND owner_id = $7
        RETURNING id, owner_id, url, title, description, author, image_url,
                  domain, reader_content, reading_time_minutes, is_favorite,
                  is_archived, tags, created_at, updated_at
        "#,
        title,
        description,
        is_favorite,
        is_archived,
        tags,
        id,
        owner.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn toggle_favorite(
    conn: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Bookmark>, sqlx::Error> {
    let row = sqlx::query_as!(
        BookmarkRow,
        r#"
        UPDATE bookmarks
        SET is_favorite = NOT is_favorite, updated_at = CAST(CURRENT_TIMESTAMP AS TEXT)
        WHERE id = $1 AND owner_id = $2
        RETURNING id, owner_id, url, title, description, author, image_url,
                  domain, reader_content, reading_time_minutes, is_favorite,
                  is_archived, tags, created_at, updated_at
        "#,
        id,
        owner.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn toggle_archived(
    conn: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Bookmark>, sqlx::Error> {
    let row = sqlx::query_as!(
        BookmarkRow,
        r#"
        UPDATE bookmarks
        SET is_archived = NOT is_archived, updated_at = CAST(CURRENT_TIMESTAMP AS TEXT)
        WHERE id = $1 AND owner_id = $2
        RETURNING id, owner_id, url, title, description, author, image_url,
                  domain, reader_content, reading_time_minutes, is_favorite,
                  is_archived, tags, created_at, updated_at
        "#,
        id,
        owner.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete(
    conn: impl PgExecutor<'_>,
    id: i64,
    owner: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "DELETE FROM bookmarks WHERE id = $1 AND owner_id = $2",
        id,
        owner.as_uuid(),
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn get_stats(pool: &PgPool, owner: &AccountId) -> Result<BookmarkStats, sqlx::Error> {
    let counts = sqlx::query!(
        r#"
        SELECT
            COUNT(*) as "total!",
            COUNT(*) FILTER (WHERE is_archived = FALSE) as "unread!",
            COUNT(*) FILTER (WHERE is_archived = TRUE) as "archived!",
            COUNT(*) FILTER (WHERE is_favorite = TRUE) as "favorite!"
        FROM bookmarks
        WHERE owner_id = $1
        "#,
        owner.as_uuid(),
    )
    .fetch_one(pool)
    .await?;

    let domain_rows = sqlx::query!(
        r#"
        SELECT domain, COUNT(*) as "cnt!"
        FROM bookmarks
        WHERE owner_id = $1
        GROUP BY domain
        ORDER BY COUNT(*) DESC
        LIMIT 10
        "#,
        owner.as_uuid(),
    )
    .fetch_all(pool)
    .await?;

    let top_domains = domain_rows
        .into_iter()
        .map(|r| DomainCount {
            domain: r.domain,
            count: r.cnt,
        })
        .collect();

    let tag_rows = sqlx::query_scalar!(
        "SELECT tags FROM bookmarks WHERE owner_id = $1 AND tags != ''",
        owner.as_uuid(),
    )
    .fetch_all(pool)
    .await?;

    let mut unique_tags = std::collections::BTreeSet::new();
    for tags_str in tag_rows {
        for t in tags_str.split(',') {
            let trimmed = t.trim();
            if !trimmed.is_empty() {
                unique_tags.insert(trimmed.to_string());
            }
        }
    }

    Ok(BookmarkStats {
        total_count: counts.total,
        unread_count: counts.unread,
        archived_count: counts.archived,
        favorite_count: counts.favorite,
        tags: unique_tags.into_iter().collect(),
        top_domains,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> (PgPool, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner =
            crate::accounts::AccountRepository::resolve_or_provision(&pool, "@alice:mbm.one")
                .await
                .unwrap();
        (pool, AccountId::new(owner))
    }

    #[tokio::test]
    async fn test_bookmark_repository_flow() {
        let (pool, owner) = setup_db().await;

        let input = CreateBookmark {
            url: "https://news.ycombinator.com".to_string(),
            title: "Hacker News".to_string(),
            description: Some("Tech news".to_string()),
            author: None,
            image_url: None,
            domain: "news.ycombinator.com".to_string(),
            reader_content: Some("Front page content".to_string()),
            reading_time_minutes: 2,
            tags: "tech,news".to_string(),
        };

        let b1 = create(&pool, &owner, &input).await.unwrap();
        assert_eq!(b1.title, "Hacker News");
        assert_eq!(b1.domain, "news.ycombinator.com");
        assert_eq!(b1.reading_time_minutes, 2);
        assert!(!b1.is_favorite);
        assert!(!b1.is_archived);

        let fav = toggle_favorite(&pool, b1.id, &owner)
            .await
            .unwrap()
            .unwrap();
        assert!(fav.is_favorite);

        let arch = toggle_archived(&pool, b1.id, &owner)
            .await
            .unwrap()
            .unwrap();
        assert!(arch.is_archived);

        let list_res = list(
            &pool,
            &owner,
            &BookmarkQuery {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(list_res.len(), 1);

        let count_res = count(
            &pool,
            &owner,
            &BookmarkQuery {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(count_res, 1);

        let stats = get_stats(&pool, &owner).await.unwrap();
        assert_eq!(stats.total_count, 1);
        assert_eq!(stats.archived_count, 1);
        assert_eq!(stats.favorite_count, 1);
        assert!(stats.tags.contains(&"tech".to_string()));

        let deleted = delete(&pool, b1.id, &owner).await.unwrap();
        assert!(deleted);
        assert!(find_by_id(&pool, b1.id, &owner).await.unwrap().is_none());
    }
}
