use anyhow::Result;
use sqlx::SqlitePool;

use super::models::{Bookmark, BookmarkQuery, BookmarkStats, DomainCount, UpdateBookmarkInput};

pub struct BookmarksRepository;

impl BookmarksRepository {
    pub async fn create(
        pool: &SqlitePool,
        owner_id: &str,
        url: &str,
        title: &str,
        description: Option<&str>,
        author: Option<&str>,
        image_url: Option<&str>,
        domain: &str,
        reader_content: Option<&str>,
        reading_time_minutes: i64,
        tags: &str,
    ) -> Result<Bookmark> {
        let bookmark = sqlx::query_as::<_, Bookmark>(
            r#"
            INSERT INTO bookmarks (
                owner_id, url, title, description, author, image_url,
                domain, reader_content, reading_time_minutes, is_favorite,
                is_archived, tags, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0, ?, datetime('now'), datetime('now'))
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
        )
        .bind(owner_id)
        .bind(url)
        .bind(title)
        .bind(description)
        .bind(author)
        .bind(image_url)
        .bind(domain)
        .bind(reader_content)
        .bind(reading_time_minutes)
        .bind(tags)
        .fetch_one(pool)
        .await?;

        Ok(bookmark)
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: i64,
        owner_id: &str,
    ) -> Result<Option<Bookmark>> {
        let row = sqlx::query_as::<_, Bookmark>(
            r#"
            SELECT id, owner_id, url, title, description, author, image_url,
                   domain, reader_content, reading_time_minutes, is_favorite,
                   is_archived, tags, created_at, updated_at
            FROM bookmarks
            WHERE id = ? AND owner_id = ?
            "#,
        )
        .bind(id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        Ok(row)
    }

    pub async fn list(
        pool: &SqlitePool,
        owner_id: &str,
        query: &BookmarkQuery,
    ) -> Result<Vec<Bookmark>> {
        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let offset = query.offset.unwrap_or(0).max(0);

        let mut sql = String::from(
            "SELECT id, owner_id, url, title, description, author, image_url, \
                    domain, reader_content, reading_time_minutes, is_favorite, \
                    is_archived, tags, created_at, updated_at \
             FROM bookmarks WHERE owner_id = ?",
        );

        if let Some(archived) = query.archived {
            sql.push_str(if archived {
                " AND is_archived = 1"
            } else {
                " AND is_archived = 0"
            });
        }
        if let Some(favorite) = query.favorite {
            sql.push_str(if favorite {
                " AND is_favorite = 1"
            } else {
                " AND is_favorite = 0"
            });
        }
        if let Some(ref domain) = query.domain {
            if !domain.trim().is_empty() {
                sql.push_str(" AND domain = ?");
            }
        }
        if let Some(ref tag) = query.tag {
            if !tag.trim().is_empty() {
                sql.push_str(" AND (tags LIKE ? OR tags = ?)");
            }
        }
        if let Some(ref search) = query.search {
            if !search.trim().is_empty() {
                sql.push_str(" AND (title LIKE ? OR description LIKE ? OR url LIKE ?)");
            }
        }

        sql.push_str(" ORDER BY id DESC LIMIT ? OFFSET ?");

        let mut q = sqlx::query_as::<_, Bookmark>(&sql).bind(owner_id);

        if let Some(ref domain) = query.domain {
            if !domain.trim().is_empty() {
                q = q.bind(domain.trim());
            }
        }
        if let Some(ref tag) = query.tag {
            let t = tag.trim();
            if !t.is_empty() {
                let pattern = format!("%{t}%");
                q = q.bind(pattern).bind(t);
            }
        }
        if let Some(ref search) = query.search {
            let s = search.trim();
            if !s.is_empty() {
                let pattern = format!("%{s}%");
                q = q.bind(pattern.clone()).bind(pattern.clone()).bind(pattern);
            }
        }

        let items = q.bind(limit).bind(offset).fetch_all(pool).await?;
        Ok(items)
    }

    pub async fn count(pool: &SqlitePool, owner_id: &str, query: &BookmarkQuery) -> Result<i64> {
        let mut sql = String::from("SELECT COUNT(*) FROM bookmarks WHERE owner_id = ?");

        if let Some(archived) = query.archived {
            sql.push_str(if archived {
                " AND is_archived = 1"
            } else {
                " AND is_archived = 0"
            });
        }
        if let Some(favorite) = query.favorite {
            sql.push_str(if favorite {
                " AND is_favorite = 1"
            } else {
                " AND is_favorite = 0"
            });
        }
        if let Some(ref domain) = query.domain {
            if !domain.trim().is_empty() {
                sql.push_str(" AND domain = ?");
            }
        }
        if let Some(ref tag) = query.tag {
            if !tag.trim().is_empty() {
                sql.push_str(" AND (tags LIKE ? OR tags = ?)");
            }
        }
        if let Some(ref search) = query.search {
            if !search.trim().is_empty() {
                sql.push_str(" AND (title LIKE ? OR description LIKE ? OR url LIKE ?)");
            }
        }

        let mut q = sqlx::query_as::<_, (i64,)>(&sql).bind(owner_id);

        if let Some(ref domain) = query.domain {
            if !domain.trim().is_empty() {
                q = q.bind(domain.trim());
            }
        }
        if let Some(ref tag) = query.tag {
            let t = tag.trim();
            if !t.is_empty() {
                let pattern = format!("%{t}%");
                q = q.bind(pattern).bind(t);
            }
        }
        if let Some(ref search) = query.search {
            let s = search.trim();
            if !s.is_empty() {
                let pattern = format!("%{s}%");
                q = q.bind(pattern.clone()).bind(pattern.clone()).bind(pattern);
            }
        }

        let (total,) = q.fetch_one(pool).await?;
        Ok(total)
    }

    pub async fn update(
        pool: &SqlitePool,
        id: i64,
        owner_id: &str,
        input: &UpdateBookmarkInput,
    ) -> Result<Bookmark> {
        let existing = Self::find_by_id(pool, id, owner_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Bookmark not found"))?;

        let title = input.title.as_deref().unwrap_or(&existing.title);
        let description = input
            .description
            .as_deref()
            .or(existing.description.as_deref());
        let is_favorite = input.is_favorite.unwrap_or(existing.is_favorite);
        let is_archived = input.is_archived.unwrap_or(existing.is_archived);
        let tags_str = input
            .tags
            .as_ref()
            .map(|t| t.join(","))
            .unwrap_or(existing.tags);

        let updated = sqlx::query_as::<_, Bookmark>(
            r#"
            UPDATE bookmarks
            SET title = ?, description = ?, is_favorite = ?, is_archived = ?, tags = ?, updated_at = datetime('now')
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
        )
        .bind(title)
        .bind(description)
        .bind(is_favorite)
        .bind(is_archived)
        .bind(tags_str)
        .bind(id)
        .bind(owner_id)
        .fetch_one(pool)
        .await?;

        Ok(updated)
    }

    pub async fn toggle_favorite(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<Bookmark> {
        let updated = sqlx::query_as::<_, Bookmark>(
            r#"
            UPDATE bookmarks
            SET is_favorite = NOT is_favorite, updated_at = datetime('now')
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(owner_id)
        .fetch_one(pool)
        .await?;

        Ok(updated)
    }

    pub async fn toggle_archived(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<Bookmark> {
        let updated = sqlx::query_as::<_, Bookmark>(
            r#"
            UPDATE bookmarks
            SET is_archived = NOT is_archived, updated_at = datetime('now')
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
        )
        .bind(id)
        .bind(owner_id)
        .fetch_one(pool)
        .await?;

        Ok(updated)
    }

    pub async fn delete(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<bool> {
        let res = sqlx::query("DELETE FROM bookmarks WHERE id = ? AND owner_id = ?")
            .bind(id)
            .bind(owner_id)
            .execute(pool)
            .await?;

        Ok(res.rows_affected() > 0)
    }

    pub async fn get_stats(pool: &SqlitePool, owner_id: &str) -> Result<BookmarkStats> {
        let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bookmarks WHERE owner_id = ?")
            .bind(owner_id)
            .fetch_one(pool)
            .await?;

        let unread: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM bookmarks WHERE owner_id = ? AND is_archived = 0")
                .bind(owner_id)
                .fetch_one(pool)
                .await?;

        let archived: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM bookmarks WHERE owner_id = ? AND is_archived = 1")
                .bind(owner_id)
                .fetch_one(pool)
                .await?;

        let favorite: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM bookmarks WHERE owner_id = ? AND is_favorite = 1")
                .bind(owner_id)
                .fetch_one(pool)
                .await?;

        let domain_rows = sqlx::query_as::<_, (String, i64)>(
            r#"
            SELECT domain, COUNT(*) as cnt
            FROM bookmarks
            WHERE owner_id = ?
            GROUP BY domain
            ORDER BY cnt DESC
            LIMIT 10
            "#,
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await?;

        let top_domains = domain_rows
            .into_iter()
            .map(|(domain, count)| DomainCount { domain, count })
            .collect();

        let tag_rows = sqlx::query_as::<_, (String,)>(
            "SELECT tags FROM bookmarks WHERE owner_id = ? AND tags != ''",
        )
        .bind(owner_id)
        .fetch_all(pool)
        .await?;

        let mut unique_tags = std::collections::BTreeSet::new();
        for (tags_str,) in tag_rows {
            for t in tags_str.split(',') {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    unique_tags.insert(trimmed.to_string());
                }
            }
        }

        Ok(BookmarkStats {
            total_count: total.0,
            unread_count: unread.0,
            archived_count: archived.0,
            favorite_count: favorite.0,
            tags: unique_tags.into_iter().collect(),
            top_domains,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> (SqlitePool, String) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&crate::db::DbPool::Sqlite(pool.clone()))
            .await
            .unwrap();
        let owner =
            crate::accounts::AccountRepository::resolve_or_provision(&pool, "@alice:mbm.one")
                .await
                .unwrap();
        (pool, owner)
    }

    #[tokio::test]
    async fn test_bookmark_crud_flow() {
        let (pool, owner) = setup_db().await;

        let b1 = BookmarksRepository::create(
            &pool,
            &owner,
            "https://news.ycombinator.com",
            "Hacker News",
            Some("Tech news"),
            None,
            None,
            "news.ycombinator.com",
            Some("Front page content"),
            2,
            "tech,news",
        )
        .await
        .unwrap();

        assert_eq!(b1.title, "Hacker News");
        assert_eq!(b1.domain, "news.ycombinator.com");
        assert!(!b1.is_favorite);
        assert!(!b1.is_archived);

        // Toggle favorite
        let fav = BookmarksRepository::toggle_favorite(&pool, b1.id, &owner)
            .await
            .unwrap();
        assert!(fav.is_favorite);

        // Toggle archived
        let arch = BookmarksRepository::toggle_archived(&pool, b1.id, &owner)
            .await
            .unwrap();
        assert!(arch.is_archived);

        // List
        let list = BookmarksRepository::list(
            &pool,
            &owner,
            &BookmarkQuery {
                archived: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(list.len(), 1);

        // Stats
        let stats = BookmarksRepository::get_stats(&pool, &owner).await.unwrap();
        assert_eq!(stats.total_count, 1);
        assert_eq!(stats.archived_count, 1);
        assert_eq!(stats.favorite_count, 1);
        assert!(stats.tags.contains(&"tech".to_string()));

        // Delete
        let deleted = BookmarksRepository::delete(&pool, b1.id, &owner)
            .await
            .unwrap();
        assert!(deleted);
        assert!(BookmarksRepository::find_by_id(&pool, b1.id, &owner)
            .await
            .unwrap()
            .is_none());
    }
}
