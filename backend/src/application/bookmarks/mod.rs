//! Bookmarks application service.
//!
//! Owns business use cases, transactions, and event publication for bookmarks.

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::bookmarks::BookmarkFields;
use crate::domain::bookmarks::{
    Bookmark, BookmarkError, BookmarkQuery, BookmarkStats, CreateBookmark, ExtractedMetadata,
    UpdateBookmark,
};
use crate::domain::events::SystemEvent;
use crate::domain::jobs::NewJob;
use crate::domain::knowledge::search::KnowledgeItemType;
use crate::infrastructure::postgres::repositories::{self, outbox};

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
}

impl BookmarkService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
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
        let mut tx = self.pool.begin().await?;
        let bookmark = repositories::bookmarks::create(&mut *tx, owner, &input).await?;
        outbox::publish(&mut tx, &SystemEvent::BookmarkCreated(bookmark.clone())).await?;
        let job = NewJob::knowledge_embed_item(*owner, KnowledgeItemType::Bookmark, bookmark.id);
        repositories::jobs::enqueue(&mut *tx, &job).await?;
        tx.commit().await?;

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

        let mut tx = self.pool.begin().await?;
        let updated = repositories::bookmarks::update_fields(
            &mut *tx,
            id,
            owner,
            BookmarkFields {
                title,
                description,
                is_favorite,
                is_archived,
                tags: &tags_str,
            },
        )
        .await?
        .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::BookmarkUpdated(updated.clone())).await?;
        tx.commit().await?;

        Ok(updated)
    }

    /// Toggles the favorite status of a bookmark.
    pub async fn toggle_favorite(&self, owner: &AccountId, id: i64) -> Result<Bookmark, AppError> {
        let mut tx = self.pool.begin().await?;
        let updated = repositories::bookmarks::toggle_favorite(&mut *tx, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::BookmarkUpdated(updated.clone())).await?;
        tx.commit().await?;

        Ok(updated)
    }

    /// Toggles the archived status of a bookmark.
    pub async fn toggle_archived(&self, owner: &AccountId, id: i64) -> Result<Bookmark, AppError> {
        let mut tx = self.pool.begin().await?;
        let updated = repositories::bookmarks::toggle_archived(&mut *tx, id, owner)
            .await?
            .ok_or_else(|| AppError::NotFound("Bookmark not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::BookmarkUpdated(updated.clone())).await?;
        tx.commit().await?;

        Ok(updated)
    }

    /// Deletes a bookmark by ID, announcing `SystemEvent::BookmarkDeleted`.
    pub async fn delete(&self, owner: &AccountId, id: i64) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repositories::bookmarks::delete(&mut *tx, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound("Bookmark not found".to_string()));
        }
        // Same transaction as the row delete, or search keeps returning a ghost.
        repositories::knowledge::delete_embeddings_for_item(
            &mut *tx,
            owner,
            KnowledgeItemType::Bookmark.as_str(),
            id,
        )
        .await?;

        outbox::publish(
            &mut tx,
            &SystemEvent::BookmarkDeleted {
                id,
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;

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

    async fn setup() -> (BookmarkService, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = crate::application::AppServices::new(&pool)
            .identity
            .provision_matrix_for_test("@alice:mbm.one")
            .await;
        let service = BookmarkService::new(pool);
        (service, owner)
    }

    async fn plant_chunk(service: &BookmarkService, owner: &AccountId, id: i64) {
        repositories::knowledge::upsert_embedding_chunk(
            &service.pool,
            owner,
            "bookmark",
            id,
            0,
            "snippet",
            &vec![0.0; 384],
            "model-a",
        )
        .await
        .unwrap();
    }

    async fn chunk_count(service: &BookmarkService, owner: &AccountId, id: i64) -> usize {
        repositories::knowledge::list_chunks(&service.pool, owner, "bookmark", id)
            .await
            .unwrap()
            .len()
    }

    /// Given an embedded bookmark, When it is deleted, Then its embeddings go with it.
    #[tokio::test]
    async fn delete_removes_embeddings() {
        let (service, owner) = setup().await;
        let b = service
            .create(
                &owner,
                "https://rust-lang.org",
                None,
                None,
                ExtractedMetadata {
                    title: "Rust".to_string(),
                    domain: "rust-lang.org".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        plant_chunk(&service, &owner, b.id).await;

        service.delete(&owner, b.id).await.unwrap();

        assert_eq!(chunk_count(&service, &owner, b.id).await, 0);
    }

    /// Given another account's bookmark, When a stranger deletes its id, Then
    /// NotFound is returned and the owner's embeddings survive.
    #[tokio::test]
    async fn delete_by_stranger_keeps_embeddings() {
        let (service, owner) = setup().await;
        let stranger = crate::application::AppServices::new(&service.pool)
            .identity
            .provision_matrix_for_test("@bob:mbm.one")
            .await;
        let b = service
            .create(
                &owner,
                "https://rust-lang.org",
                None,
                None,
                ExtractedMetadata {
                    title: "Rust".to_string(),
                    domain: "rust-lang.org".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        plant_chunk(&service, &owner, b.id).await;

        let err = service.delete(&stranger, b.id).await.unwrap_err();

        assert!(matches!(err, AppError::NotFound(_)));
        assert_eq!(chunk_count(&service, &owner, b.id).await, 1);
    }

    #[tokio::test]
    async fn test_service_crud_and_events() {
        let (service, owner) = setup().await;

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

        let fav = service.toggle_favorite(&owner, b.id).await.unwrap();
        assert!(fav.is_favorite);

        service.delete(&owner, b.id).await.unwrap();

        assert_eq!(
            repositories::outbox::event_types_for(&service.pool, &owner)
                .await
                .unwrap(),
            vec!["BookmarkCreated", "BookmarkUpdated", "BookmarkDeleted"]
        );
    }

    /// Given an owner, When a bookmark is created, Then BookmarkCreated is in the outbox.
    #[tokio::test]
    async fn create_writes_bookmark_created_to_the_outbox() {
        let (service, owner) = setup().await;

        service
            .create(
                &owner,
                "https://rust-lang.org",
                None,
                None,
                ExtractedMetadata {
                    title: "Rust".to_string(),
                    domain: "rust-lang.org".to_string(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(
            repositories::outbox::event_types_for(&service.pool, &owner)
                .await
                .unwrap(),
            vec!["BookmarkCreated"]
        );
    }

    /// Given the outbox insert failing, When create runs, Then the bookmark
    /// row is rolled back and the call errors.
    #[tokio::test]
    async fn a_failed_create_leaves_no_row_and_no_event() {
        let (service, owner) = setup().await;
        repositories::outbox::fail_every_publish(&service.pool).await;

        assert!(service
            .create(
                &owner,
                "https://rust-lang.org",
                Some("title"),
                None,
                ExtractedMetadata::default(),
            )
            .await
            .is_err());

        assert_eq!(
            service
                .count(&owner, &BookmarkQuery::default())
                .await
                .unwrap(),
            0
        );
        assert!(repositories::outbox::event_types_for(&service.pool, &owner)
            .await
            .unwrap()
            .is_empty());
    }
}
