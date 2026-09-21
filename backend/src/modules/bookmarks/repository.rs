use crate::db::DbPool;
use anyhow::Result;
use sqlx::SqlitePool;

use super::models::{Bookmark, BookmarkQuery, BookmarkStats, DomainCount, UpdateBookmarkInput};

pub struct BookmarksRepository;

impl BookmarksRepository {
    pub async fn create(
        pool: &DbPool,
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
        let bookmark = crate::db_fetch_one!(
            pool,
            Bookmark,
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
            owner_id,
            url,
            title,
            description,
            author,
            image_url,
            domain,
            reader_content,
            reading_time_minutes,
            tags
        )?;

        Ok(bookmark)
    }

    pub async fn find_by_id(pool: &DbPool, id: i64, owner_id: &str) -> Result<Option<Bookmark>> {
        let row = crate::db_fetch_optional!(
            pool,
            Bookmark,
            r#"
            SELECT id, owner_id, url, title, description, author, image_url,
                   domain, reader_content, reading_time_minutes, is_favorite,
                   is_archived, tags, created_at, updated_at
            FROM bookmarks
            WHERE id = ? AND owner_id = ?
            "#,
            id,
            owner_id
        )?;

        Ok(row)
    }

    pub async fn list(
        pool: &DbPool,
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
                " AND is_archived = TRUE"
            } else {
                " AND is_archived = FALSE"
            });
        }
        if let Some(favorite) = query.favorite {
            sql.push_str(if favorite {
                " AND is_favorite = TRUE"
            } else {
                " AND is_favorite = FALSE"
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

        // The filters are optional, so the bind count is decided here and the
        // values are handed to `db_fetch_all_dyn!` in the same order the `WHERE`
        // clause above was assembled. `limit`/`offset` stay fixed trailing binds.
        let mut binds: Vec<String> = vec![owner_id.to_string()];

        if let Some(ref domain) = query.domain {
            if !domain.trim().is_empty() {
                binds.push(domain.trim().to_string());
            }
        }
        if let Some(ref tag) = query.tag {
            let t = tag.trim();
            if !t.is_empty() {
                binds.push(format!("%{t}%"));
                binds.push(t.to_string());
            }
        }
        if let Some(ref search) = query.search {
            let s = search.trim();
            if !s.is_empty() {
                let pattern = format!("%{s}%");
                binds.push(pattern.clone());
                binds.push(pattern.clone());
                binds.push(pattern);
            }
        }

        let items = crate::db_fetch_all_dyn!(pool, Bookmark, &sql, binds, limit, offset)?;
        Ok(items)
    }

    pub async fn count(pool: &DbPool, owner_id: &str, query: &BookmarkQuery) -> Result<i64> {
        let mut sql = String::from("SELECT COUNT(*) FROM bookmarks WHERE owner_id = ?");

        if let Some(archived) = query.archived {
            sql.push_str(if archived {
                " AND is_archived = TRUE"
            } else {
                " AND is_archived = FALSE"
            });
        }
        if let Some(favorite) = query.favorite {
            sql.push_str(if favorite {
                " AND is_favorite = TRUE"
            } else {
                " AND is_favorite = FALSE"
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

        // Same filter order as `list` above; see the note there.
        let mut binds: Vec<String> = vec![owner_id.to_string()];

        if let Some(ref domain) = query.domain {
            if !domain.trim().is_empty() {
                binds.push(domain.trim().to_string());
            }
        }
        if let Some(ref tag) = query.tag {
            let t = tag.trim();
            if !t.is_empty() {
                binds.push(format!("%{t}%"));
                binds.push(t.to_string());
            }
        }
        if let Some(ref search) = query.search {
            let s = search.trim();
            if !s.is_empty() {
                let pattern = format!("%{s}%");
                binds.push(pattern.clone());
                binds.push(pattern.clone());
                binds.push(pattern);
            }
        }

        let (total,) = crate::db_fetch_one_dyn!(pool, (i64,), &sql, binds)?;
        Ok(total)
    }

    pub async fn update(
        pool: &DbPool,
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

        let updated = crate::db_fetch_one!(
            pool,
            Bookmark,
            r#"
            UPDATE bookmarks
            SET title = ?, description = ?, is_favorite = ?, is_archived = ?, tags = ?, updated_at = datetime('now')
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
            title,
            description,
            is_favorite,
            is_archived,
            tags_str,
            id,
            owner_id
        )?;

        Ok(updated)
    }

    pub async fn toggle_favorite(pool: &DbPool, id: i64, owner_id: &str) -> Result<Bookmark> {
        let updated = crate::db_fetch_one!(
            pool,
            Bookmark,
            r#"
            UPDATE bookmarks
            SET is_favorite = NOT is_favorite, updated_at = datetime('now')
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
            id,
            owner_id
        )?;

        Ok(updated)
    }

    pub async fn toggle_archived(pool: &DbPool, id: i64, owner_id: &str) -> Result<Bookmark> {
        let updated = crate::db_fetch_one!(
            pool,
            Bookmark,
            r#"
            UPDATE bookmarks
            SET is_archived = NOT is_archived, updated_at = datetime('now')
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, url, title, description, author, image_url,
                      domain, reader_content, reading_time_minutes, is_favorite,
                      is_archived, tags, created_at, updated_at
            "#,
            id,
            owner_id
        )?;

        Ok(updated)
    }

    pub async fn delete(pool: &DbPool, id: i64, owner_id: &str) -> Result<bool> {
        let res = crate::db_execute!(
            pool,
            "DELETE FROM bookmarks WHERE id = ? AND owner_id = ?",
            id,
            owner_id
        )?;

        Ok(res > 0)
    }

    pub async fn get_stats(pool: &DbPool, owner_id: &str) -> Result<BookmarkStats> {
        let total: (i64,) = crate::db_fetch_one!(
            pool,
            _,
            "SELECT COUNT(*) FROM bookmarks WHERE owner_id = ?",
            owner_id
        )?;

        let unread: (i64,) = crate::db_fetch_one!(
            pool,
            _,
            "SELECT COUNT(*) FROM bookmarks WHERE owner_id = ? AND is_archived = FALSE",
            owner_id
        )?;

        let archived: (i64,) = crate::db_fetch_one!(
            pool,
            _,
            "SELECT COUNT(*) FROM bookmarks WHERE owner_id = ? AND is_archived = TRUE",
            owner_id
        )?;

        let favorite: (i64,) = crate::db_fetch_one!(
            pool,
            _,
            "SELECT COUNT(*) FROM bookmarks WHERE owner_id = ? AND is_favorite = TRUE",
            owner_id
        )?;

        let domain_rows = crate::db_fetch_all!(
            pool,
            (String, i64),
            r#"
            SELECT domain, COUNT(*) as cnt
            FROM bookmarks
            WHERE owner_id = ?
            GROUP BY domain
            ORDER BY cnt DESC
            LIMIT 10
            "#,
            owner_id
        )?;

        let top_domains = domain_rows
            .into_iter()
            .map(|(domain, count)| DomainCount { domain, count })
            .collect();

        let tag_rows = crate::db_fetch_all!(
            pool,
            (String,),
            "SELECT tags FROM bookmarks WHERE owner_id = ? AND tags != ''",
            owner_id
        )?;

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

    async fn setup_db() -> (DbPool, String) {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db::schema::run_migrations(&pool).await.unwrap();
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
