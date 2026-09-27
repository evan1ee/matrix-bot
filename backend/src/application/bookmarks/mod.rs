//! Bookmarks application service.
//!
//! Owns business use cases, transactions, and event publication for bookmarks.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::bookmarks::{
    Bookmark, BookmarkError, BookmarkQuery, BookmarkStats, CreateBookmark, ExtractedMetadata,
    UpdateBookmark,
};
use crate::events::{EventBroadcaster, SystemEvent};
use crate::infrastructure::postgres::repositories;

impl From<BookmarkError> for AppError {
    fn from(err: BookmarkError) -> Self {
        match err {
            BookmarkError::NotFound => AppError::NotFound("Bookmark not found".to_string()),
            other => AppError::Validation(other.to_string()),
        }
    }
}

#[derive(Clone)]
pub struct BookmarkService {
    pool: PgPool,
    events: EventBroadcaster,
}

impl BookmarkService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self { pool, events }
    }

    /// Creates and persists a new bookmark, then announces `SystemEvent::BookmarkCreated`.
    pub async fn create(
        &self,
        owner: &AccountId,
        url: &str,
        user_title: Option<&str>,
        tags: Option<&[String]>,
        meta: ExtractedMetadata,
    ) -> Result<Bookmark, AppError> {
        let input = CreateBookmark::new(url, meta, user_title, tags)?;
        let bookmark = repositories::bookmarks::create(&self.pool, owner, &input).await?;

        self.events
            .send(SystemEvent::BookmarkCreated(bookmark.clone()));

        Ok(bookmark)
    }

    /// Returns a bookmark by ID if owned by `owner`.
    pub async fn get(&self, owner: &AccountId, id: i64) -> Result<Bookmark, AppError> {
        repositories::bookmarks::find_by_id(&self.pool, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))
    }

    /// Lists bookmarks matching the query filters.
    pub async fn list(
        &self,
        owner: &AccountId,
        query: &BookmarkQuery,
    ) -> Result<Vec<Bookmark>, AppError> {
        let items = repositories::bookmarks::list(&self.pool, owner, query).await?;
        Ok(items)
    }

    /// Counts bookmarks matching the query filters.
    pub async fn count(&self, owner: &AccountId, query: &BookmarkQuery) -> Result<i64, AppError> {
        let total = repositories::bookmarks::count(&self.pool, owner, query).await?;
        Ok(total)
    }

    /// Updates fields on an existing bookmark, announcing `SystemEvent::BookmarkUpdated`.
    pub async fn update(
        &self,
        owner: &AccountId,
        id: i64,
        input: UpdateBookmark,
    ) -> Result<Bookmark, AppError> {
        input.validate()?;

        let existing = self.get(owner, id).await?;

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

        let updated = repositories::bookmarks::update_fields(
            &self.pool,
            id,
            owner,
            title,
            description,
            is_favorite,
            is_archived,
            &tags_str,
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))?;

        self.events
            .send(SystemEvent::BookmarkUpdated(updated.clone()));

        Ok(updated)
    }

    /// Toggles the favorite status of a bookmark.
    pub async fn toggle_favorite(&self, owner: &AccountId, id: i64) -> Result<Bookmark, AppError> {
        let updated = repositories::bookmarks::toggle_favorite(&self.pool, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))?;

        self.events
            .send(SystemEvent::BookmarkUpdated(updated.clone()));

        Ok(updated)
    }

    /// Toggles the archived status of a bookmark.
    pub async fn toggle_archived(&self, owner: &AccountId, id: i64) -> Result<Bookmark, AppError> {
        let updated = repositories::bookmarks::toggle_archived(&self.pool, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))?;

        self.events
            .send(SystemEvent::BookmarkUpdated(updated.clone()));

        Ok(updated)
    }

    /// Deletes a bookmark by ID, announcing `SystemEvent::BookmarkDeleted`.
    pub async fn delete(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let deleted = repositories::bookmarks::delete(&self.pool, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound("Bookmark not found".to_string()));
        }

        self.events.send(SystemEvent::BookmarkDeleted {
            id,
            owner_id: owner.to_string(),
        });

        Ok(())
    }

    /// Gathers statistics on bookmarks for the specified owner.
    pub async fn get_stats(&self, owner: &AccountId) -> Result<BookmarkStats, AppError> {
        let stats = repositories::bookmarks::get_stats(&self.pool, owner).await?;
        Ok(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup() -> (BookmarkService, AccountId, EventBroadcaster) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner =
            crate::accounts::AccountRepository::resolve_or_provision(&pool, "@alice:mbm.one")
                .await
                .unwrap();
        let events = EventBroadcaster::default();
        let service = BookmarkService::new(pool, events.clone());
        (service, AccountId::new(owner), events)
    }

    #[tokio::test]
    async fn test_service_crud_and_events() {
        let (service, owner, events) = setup().await;
        let mut rx = events.subscribe();

        let meta = ExtractedMetadata {
            title: "Rust Programming".to_string(),
            domain: "rust-lang.org".to_string(),
            ..Default::default()
        };

        // Create
        let b = service
            .create(
                &owner,
                "https://rust-lang.org",
                None,
                Some(&["lang".to_string(), "systems".to_string()]),
                meta,
            )
            .await
            .unwrap();

        match rx.recv().await.unwrap() {
            SystemEvent::BookmarkCreated(created) => {
                assert_eq!(created.id, b.id);
                assert_eq!(created.owner_id, owner);
            }
            other => panic!("Unexpected event: {other:?}"),
        }

        // Toggle favorite
        let fav = service.toggle_favorite(&owner, b.id).await.unwrap();
        assert!(fav.is_favorite);
        match rx.recv().await.unwrap() {
            SystemEvent::BookmarkUpdated(updated) => {
                assert_eq!(updated.id, b.id);
                assert!(updated.is_favorite);
            }
            other => panic!("Unexpected event: {other:?}"),
        }

        // Delete
        service.delete(&owner, b.id).await.unwrap();
        match rx.recv().await.unwrap() {
            SystemEvent::BookmarkDeleted { id, owner_id } => {
                assert_eq!(id, b.id);
                assert_eq!(owner_id, owner.as_str());
            }
            other => panic!("Unexpected event: {other:?}"),
        }
    }
}
