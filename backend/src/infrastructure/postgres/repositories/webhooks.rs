//! PostgreSQL repository for webhooks (alert deliveries live in the outbox).

use chrono::{DateTime, Utc};
use sqlx::{PgExecutor, PgPool};

use crate::domain::accounts::AccountId;
use crate::domain::webhooks::Webhook;

#[derive(Debug, sqlx::FromRow)]
struct WebhookRow {
    id: String,
    owner_id: uuid::Uuid,
    name: String,
    token_prefix: String,
    default_room_id: String,
    is_active: bool,
    total_delivered_count: i64,
    failure_count: i64,
    last_triggered_at: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
}

impl From<WebhookRow> for Webhook {
    fn from(r: WebhookRow) -> Self {
        Self {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id),
            name: r.name,
            token_prefix: r.token_prefix,
            default_room_id: r.default_room_id,
            is_active: r.is_active,
            total_delivered_count: r.total_delivered_count,
            failure_count: r.failure_count,
            last_triggered_at: r.last_triggered_at,
            created_at: r.created_at.unwrap_or_else(Utc::now),
        }
    }
}

pub async fn create(
    ex: impl PgExecutor<'_>,
    id: &str,
    owner_id: &AccountId,
    name: &str,
    token_hash: &str,
    token_prefix: &str,
    default_room_id: &str,
) -> Result<Webhook, sqlx::Error> {
    let row = sqlx::query_as!(
        WebhookRow,
        r#"
        INSERT INTO webhooks (
            id, owner_id, name, token_hash, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, TRUE, 0, 0, CURRENT_TIMESTAMP)
        RETURNING
            id, owner_id, name, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count,
            last_triggered_at, created_at
        "#,
        id,
        owner_id.as_uuid(),
        name,
        token_hash,
        token_prefix,
        default_room_id
    )
    .fetch_one(ex)
    .await?;

    Ok(row.into())
}

pub async fn find_by_token_hash(
    pool: &PgPool,
    token_hash: &str,
) -> Result<Option<Webhook>, sqlx::Error> {
    let opt = sqlx::query_as!(
        WebhookRow,
        r#"
        SELECT
            id, owner_id, name, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count,
            last_triggered_at, created_at
        FROM webhooks
        WHERE token_hash = $1 AND is_active = TRUE
        "#,
        token_hash
    )
    .fetch_optional(pool)
    .await?;

    Ok(opt.map(Into::into))
}

pub async fn find_by_id(
    pool: &PgPool,
    id: &str,
    owner_id: &AccountId,
) -> Result<Option<Webhook>, sqlx::Error> {
    let opt = sqlx::query_as!(
        WebhookRow,
        r#"
        SELECT
            id, owner_id, name, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count,
            last_triggered_at, created_at
        FROM webhooks
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(pool)
    .await?;

    Ok(opt.map(Into::into))
}

pub async fn list_by_owner(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<Vec<Webhook>, sqlx::Error> {
    let rows = sqlx::query_as!(
        WebhookRow,
        r#"
        SELECT
            id, owner_id, name, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count,
            last_triggered_at, created_at
        FROM webhooks
        WHERE owner_id = $1
        ORDER BY created_at DESC
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update(
    ex: impl PgExecutor<'_>,
    id: &str,
    owner_id: &AccountId,
    name: Option<&str>,
    default_room_id: Option<&str>,
    is_active: Option<bool>,
) -> Result<Option<Webhook>, sqlx::Error> {
    let opt = sqlx::query_as!(
        WebhookRow,
        r#"
        UPDATE webhooks
        SET
            name = COALESCE($1, name),
            default_room_id = COALESCE($2, default_room_id),
            is_active = COALESCE($3, is_active)
        WHERE id = $4 AND owner_id = $5
        RETURNING
            id, owner_id, name, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count,
            last_triggered_at, created_at
        "#,
        name,
        default_room_id,
        is_active,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(ex)
    .await?;

    Ok(opt.map(Into::into))
}

pub async fn rotate_token(
    ex: impl PgExecutor<'_>,
    id: &str,
    owner_id: &AccountId,
    token_hash: &str,
    token_prefix: &str,
) -> Result<Option<Webhook>, sqlx::Error> {
    let opt = sqlx::query_as!(
        WebhookRow,
        r#"
        UPDATE webhooks
        SET token_hash = $1, token_prefix = $2
        WHERE id = $3 AND owner_id = $4
        RETURNING
            id, owner_id, name, token_prefix, default_room_id,
            is_active, total_delivered_count, failure_count,
            last_triggered_at, created_at
        "#,
        token_hash,
        token_prefix,
        id,
        owner_id.as_uuid()
    )
    .fetch_optional(ex)
    .await?;

    Ok(opt.map(Into::into))
}

pub async fn delete(
    ex: impl PgExecutor<'_>,
    id: &str,
    owner_id: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "DELETE FROM webhooks WHERE id = $1 AND owner_id = $2",
        id,
        owner_id.as_uuid()
    )
    .execute(ex)
    .await?;

    Ok(res.rows_affected() > 0)
}

/// Stamps the webhook when an alert is accepted; runs in the transaction
/// that publishes the alert.
pub async fn touch_last_triggered(
    ex: impl PgExecutor<'_>,
    webhook_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE webhooks SET last_triggered_at = CURRENT_TIMESTAMP WHERE id = $1",
        webhook_id
    )
    .execute(ex)
    .await?;
    Ok(())
}

pub async fn increment_delivered(
    ex: impl PgExecutor<'_>,
    webhook_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE webhooks SET total_delivered_count = total_delivered_count + 1 WHERE id = $1",
        webhook_id
    )
    .execute(ex)
    .await?;
    Ok(())
}

/// Counts every failed attempt, retry or final (C1).
pub async fn increment_failures(
    ex: impl PgExecutor<'_>,
    webhook_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE webhooks SET failure_count = failure_count + 1 WHERE id = $1",
        webhook_id
    )
    .execute(ex)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::webhooks::hash_token;
    use crate::infrastructure::postgres::repositories::accounts::core as accounts_repo;

    async fn setup_db() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_webhooks_repository_crud() {
        let pool = setup_db().await;
        let owner = accounts_repo::create(&pool, "test_whk", STATUS_ACTIVE)
            .await
            .unwrap();

        let id = "whk_test_1";
        let raw_token = "whk_random_secret_string";
        let token_hash = hash_token(raw_token);
        let token_prefix = "whk_random";

        // Create
        let created = create(
            &pool,
            id,
            &owner,
            "VPS Alerts",
            &token_hash,
            token_prefix,
            "!ops:matrix.org",
        )
        .await
        .unwrap();
        assert_eq!(created.id, id);
        assert_eq!(created.name, "VPS Alerts");
        assert_eq!(created.default_room_id, "!ops:matrix.org");

        // Find by token hash
        let found_by_hash = find_by_token_hash(&pool, &token_hash)
            .await
            .unwrap()
            .expect("should find by token hash");
        assert_eq!(found_by_hash.id, id);

        // Find by ID
        let found = find_by_id(&pool, id, &owner)
            .await
            .unwrap()
            .expect("should find by id");
        assert_eq!(found.id, id);

        // List
        let list = list_by_owner(&pool, &owner).await.unwrap();
        assert_eq!(list.len(), 1);

        // Update
        let updated = update(
            &pool,
            id,
            &owner,
            Some("New VPS Alerts"),
            Some("!ops2:matrix.org"),
            Some(true),
        )
        .await
        .unwrap()
        .expect("should update");
        assert_eq!(updated.name, "New VPS Alerts");
        assert_eq!(updated.default_room_id, "!ops2:matrix.org");

        // Rotate token
        let new_hash = hash_token("whk_new_secret");
        let rotated = rotate_token(&pool, id, &owner, &new_hash, "whk_new_se")
            .await
            .unwrap()
            .expect("should rotate");
        assert_eq!(rotated.token_prefix, "whk_new_se");

        // Counters
        touch_last_triggered(&pool, id).await.unwrap();
        increment_delivered(&pool, id).await.unwrap();
        increment_failures(&pool, id).await.unwrap();
        increment_failures(&pool, id).await.unwrap();
        let counted = find_by_id(&pool, id, &owner).await.unwrap().unwrap();
        assert_eq!(counted.total_delivered_count, 1);
        assert_eq!(counted.failure_count, 2);
        assert!(counted.last_triggered_at.is_some());

        // Delete
        let deleted = delete(&pool, id, &owner).await.unwrap();
        assert!(deleted);
        let none = find_by_id(&pool, id, &owner).await.unwrap();
        assert!(none.is_none());
    }
}
