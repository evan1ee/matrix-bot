//! PostgreSQL repository for webhooks and the webhook outbox.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::AccountId;
use crate::domain::webhooks::{Webhook, WebhookOutboxItem};

#[derive(Debug, sqlx::FromRow)]
struct WebhookRow {
    id: String,
    owner_id: String,
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
            owner_id: AccountId::from(r.owner_id),
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

#[derive(Debug, sqlx::FromRow)]
struct WebhookOutboxRow {
    id: String,
    webhook_id: String,
    owner_id: String,
    room_id: String,
    severity: String,
    title: Option<String>,
    html_body: String,
    plain_body: String,
    status: String,
    attempts: i64,
    max_attempts: i64,
    next_attempt_at: DateTime<Utc>,
    last_error: Option<String>,
    created_at: Option<DateTime<Utc>>,
    delivered_at: Option<DateTime<Utc>>,
}

impl From<WebhookOutboxRow> for WebhookOutboxItem {
    fn from(r: WebhookOutboxRow) -> Self {
        Self {
            id: r.id,
            webhook_id: r.webhook_id,
            owner_id: AccountId::from(r.owner_id),
            room_id: r.room_id,
            severity: r.severity,
            title: r.title,
            html_body: r.html_body,
            plain_body: r.plain_body,
            status: r.status,
            attempts: r.attempts,
            max_attempts: r.max_attempts,
            next_attempt_at: r.next_attempt_at,
            last_error: r.last_error,
            created_at: r.created_at.unwrap_or_else(Utc::now),
            delivered_at: r.delivered_at,
        }
    }
}

pub async fn create(
    pool: &PgPool,
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
        owner_id.as_str(),
        name,
        token_hash,
        token_prefix,
        default_room_id
    )
    .fetch_one(pool)
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
        owner_id.as_str()
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
        owner_id.as_str()
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update(
    pool: &PgPool,
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
        owner_id.as_str()
    )
    .fetch_optional(pool)
    .await?;

    Ok(opt.map(Into::into))
}

pub async fn rotate_token(
    pool: &PgPool,
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
        owner_id.as_str()
    )
    .fetch_optional(pool)
    .await?;

    Ok(opt.map(Into::into))
}

pub async fn delete(pool: &PgPool, id: &str, owner_id: &AccountId) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "DELETE FROM webhooks WHERE id = $1 AND owner_id = $2",
        id,
        owner_id.as_str()
    )
    .execute(pool)
    .await?;

    Ok(res.rows_affected() > 0)
}

#[allow(clippy::too_many_arguments)]
pub async fn enqueue_outbox(
    pool: &PgPool,
    id: &str,
    webhook_id: &str,
    owner_id: &AccountId,
    room_id: &str,
    severity: &str,
    title: Option<&str>,
    html_body: &str,
    plain_body: &str,
) -> Result<WebhookOutboxItem, sqlx::Error> {
    let now = Utc::now();

    let row = sqlx::query_as!(
        WebhookOutboxRow,
        r#"
        INSERT INTO webhook_outbox (
            id, webhook_id, owner_id, room_id, severity, title,
            html_body, plain_body, status, attempts, max_attempts,
            next_attempt_at, created_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'pending', 0, 10, $9, $10)
        RETURNING
            id, webhook_id, owner_id, room_id, severity, title,
            html_body, plain_body, status, attempts, max_attempts,
            next_attempt_at, last_error, created_at, delivered_at
        "#,
        id,
        webhook_id,
        owner_id.as_str(),
        room_id,
        severity,
        title,
        html_body,
        plain_body,
        now,
        now
    )
    .fetch_one(pool)
    .await?;

    sqlx::query!(
        "UPDATE webhooks SET last_triggered_at = CURRENT_TIMESTAMP WHERE id = $1",
        webhook_id
    )
    .execute(pool)
    .await?;

    Ok(row.into())
}

pub async fn fetch_pending_outbox(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<WebhookOutboxItem>, sqlx::Error> {
    let now = Utc::now();
    let rows = sqlx::query_as!(
        WebhookOutboxRow,
        r#"
        SELECT
            id, webhook_id, owner_id, room_id, severity, title,
            html_body, plain_body, status, attempts, max_attempts,
            next_attempt_at, last_error, created_at, delivered_at
        FROM webhook_outbox
        WHERE status IN ('pending', 'sending') AND next_attempt_at <= $1
        ORDER BY created_at ASC
        LIMIT $2
        "#,
        now,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn mark_outbox_delivered(
    pool: &PgPool,
    id: &str,
    webhook_id: &str,
) -> Result<(), sqlx::Error> {
    let now = Utc::now();
    sqlx::query!(
        r#"
        UPDATE webhook_outbox
        SET status = 'delivered', delivered_at = $1
        WHERE id = $2
        "#,
        now,
        id
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        "UPDATE webhooks SET total_delivered_count = total_delivered_count + 1 WHERE id = $1",
        webhook_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_outbox_retry(
    pool: &PgPool,
    id: &str,
    webhook_id: &str,
    attempts: i64,
    next_attempt_at: DateTime<Utc>,
    last_error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        UPDATE webhook_outbox
        SET status = 'pending', attempts = $1, next_attempt_at = $2, last_error = $3
        WHERE id = $4
        "#,
        attempts,
        next_attempt_at,
        last_error,
        id
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        "UPDATE webhooks SET failure_count = failure_count + 1 WHERE id = $1",
        webhook_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn mark_outbox_failed(
    pool: &PgPool,
    id: &str,
    webhook_id: &str,
    last_error: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        r#"
        UPDATE webhook_outbox
        SET status = 'failed', last_error = $1
        WHERE id = $2
        "#,
        last_error,
        id
    )
    .execute(pool)
    .await?;

    sqlx::query!(
        "UPDATE webhooks SET failure_count = failure_count + 1 WHERE id = $1",
        webhook_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn list_outbox_by_webhook(
    pool: &PgPool,
    webhook_id: &str,
    owner_id: &AccountId,
    limit: i64,
    offset: i64,
) -> Result<Vec<WebhookOutboxItem>, sqlx::Error> {
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    let rows = sqlx::query_as!(
        WebhookOutboxRow,
        r#"
        SELECT
            id, webhook_id, owner_id, room_id, severity, title,
            html_body, plain_body, status, attempts, max_attempts,
            next_attempt_at, last_error, created_at, delivered_at
        FROM webhook_outbox
        WHERE webhook_id = $1 AND owner_id = $2
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
        webhook_id,
        owner_id.as_str(),
        limit,
        offset
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count_outbox_by_webhook(
    pool: &PgPool,
    webhook_id: &str,
    owner_id: &AccountId,
) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM webhook_outbox WHERE webhook_id = $1 AND owner_id = $2",
        webhook_id,
        owner_id.as_str()
    )
    .fetch_one(pool)
    .await?;

    Ok(count.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::accounts::{AccountRepository, STATUS_ACTIVE};
    use crate::domain::webhooks::hash_token;

    async fn setup_db() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_webhooks_repository_crud() {
        let pool = setup_db().await;
        let acct_str = AccountRepository::create(&pool, "test_whk", STATUS_ACTIVE)
            .await
            .unwrap();
        let owner = AccountId::from(acct_str);

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

        // Outbox enqueue
        let outbox_id = "outbox_test_1";
        let outbox = enqueue_outbox(
            &pool,
            outbox_id,
            id,
            &owner,
            "!ops2:matrix.org",
            "critical",
            Some("Disk Full"),
            "<b>Disk Full</b>",
            "Disk Full",
        )
        .await
        .unwrap();
        assert_eq!(outbox.id, outbox_id);
        assert_eq!(outbox.status, "pending");

        // Pending outbox
        let pending = fetch_pending_outbox(&pool, 10).await.unwrap();
        assert_eq!(pending.len(), 1);

        // List outbox & count
        let logs = list_outbox_by_webhook(&pool, id, &owner, 10, 0)
            .await
            .unwrap();
        assert_eq!(logs.len(), 1);
        let count = count_outbox_by_webhook(&pool, id, &owner).await.unwrap();
        assert_eq!(count, 1);

        // Mark delivered
        mark_outbox_delivered(&pool, outbox_id, id).await.unwrap();
        let pending_after = fetch_pending_outbox(&pool, 10).await.unwrap();
        assert_eq!(pending_after.len(), 0);

        // Delete
        let deleted = delete(&pool, id, &owner).await.unwrap();
        assert!(deleted);
        let none = find_by_id(&pool, id, &owner).await.unwrap();
        assert!(none.is_none());
    }
}
