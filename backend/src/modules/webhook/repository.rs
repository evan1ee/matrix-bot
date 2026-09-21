use crate::db::DbPool;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::modules::webhook::models::{Webhook, WebhookOutboxItem};

/// Hashes an inbound webhook token to SHA-256 for secure database lookup.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest
        .iter()
        .fold(String::with_capacity(64), |mut acc, byte| {
            use std::fmt::Write;
            let _ = write!(acc, "{byte:02x}");
            acc
        })
}

pub struct WebhookRepository;

impl WebhookRepository {
    /// Creates a new webhook, generating an unhashed token returned once.
    pub async fn create(
        pool: &DbPool,
        owner_id: &str,
        name: &str,
        default_room_id: &str,
    ) -> Result<(Webhook, String), sqlx::Error> {
        let raw_random = crate::db_scalar_one!(pool, String, "SELECT lower(hex(randomblob(24)))")?;
        let raw_token = format!("whk_{raw_random}");
        let token_hash = hash_token(&raw_token);
        let token_prefix = raw_token.chars().take(10).collect::<String>();
        let id = format!("whk_{}", Uuid::new_v4());

        let webhook = crate::db_fetch_one!(
            pool,
            Webhook,
            r#"
            INSERT INTO webhooks (
                id, owner_id, name, token_hash, token_prefix, default_room_id,
                is_active, total_delivered_count, failure_count, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, 1, 0, 0, CURRENT_TIMESTAMP)
            RETURNING *;
            "#,
            id,
            owner_id,
            name,
            token_hash,
            token_prefix,
            default_room_id
        )?;

        Ok((webhook, raw_token))
    }

    pub async fn find_by_token_hash(
        pool: &DbPool,
        token_hash: &str,
    ) -> Result<Option<Webhook>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            Webhook,
            "SELECT * FROM webhooks WHERE token_hash = ? AND is_active = 1;",
            token_hash
        )
    }

    pub async fn find_by_id(
        pool: &DbPool,
        id: &str,
        owner_id: &str,
    ) -> Result<Option<Webhook>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            Webhook,
            "SELECT * FROM webhooks WHERE id = ? AND owner_id = ?;",
            id,
            owner_id
        )
    }

    pub async fn list_by_owner(pool: &DbPool, owner_id: &str) -> Result<Vec<Webhook>, sqlx::Error> {
        crate::db_fetch_all!(
            pool,
            Webhook,
            "SELECT * FROM webhooks WHERE owner_id = ? ORDER BY created_at DESC;",
            owner_id
        )
    }

    pub async fn update(
        pool: &DbPool,
        id: &str,
        owner_id: &str,
        name: Option<&str>,
        default_room_id: Option<&str>,
        is_active: Option<bool>,
    ) -> Result<Option<Webhook>, sqlx::Error> {
        let existing = match Self::find_by_id(pool, id, owner_id).await? {
            Some(w) => w,
            None => return Ok(None),
        };

        let new_name = name.unwrap_or(&existing.name);
        let new_room = default_room_id.unwrap_or(&existing.default_room_id);
        let new_active = is_active.unwrap_or(existing.is_active);

        let updated = crate::db_fetch_optional!(
            pool,
            Webhook,
            r#"
            UPDATE webhooks
            SET name = ?, default_room_id = ?, is_active = ?
            WHERE id = ? AND owner_id = ?
            RETURNING *;
            "#,
            new_name,
            new_room,
            new_active,
            id,
            owner_id
        )?;

        Ok(updated)
    }

    pub async fn rotate_token(
        pool: &DbPool,
        id: &str,
        owner_id: &str,
    ) -> Result<Option<(Webhook, String)>, sqlx::Error> {
        let raw_random = crate::db_scalar_one!(pool, String, "SELECT lower(hex(randomblob(24)))")?;
        let raw_token = format!("whk_{raw_random}");
        let token_hash = hash_token(&raw_token);
        let token_prefix = raw_token.chars().take(10).collect::<String>();

        let updated = crate::db_fetch_optional!(
            pool,
            Webhook,
            r#"
            UPDATE webhooks
            SET token_hash = ?, token_prefix = ?
            WHERE id = ? AND owner_id = ?
            RETURNING *;
            "#,
            token_hash,
            token_prefix,
            id,
            owner_id
        )?;

        Ok(updated.map(|w| (w, raw_token)))
    }

    pub async fn delete(pool: &DbPool, id: &str, owner_id: &str) -> Result<bool, sqlx::Error> {
        let res = crate::db_execute!(
            pool,
            "DELETE FROM webhooks WHERE id = ? AND owner_id = ?;",
            id,
            owner_id
        )?;
        Ok(res > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn enqueue_outbox(
        pool: &DbPool,
        webhook_id: &str,
        owner_id: &str,
        room_id: &str,
        severity: &str,
        title: Option<&str>,
        html_body: &str,
        plain_body: &str,
    ) -> Result<WebhookOutboxItem, sqlx::Error> {
        let id = format!("outbox_{}", Uuid::new_v4());
        let now = Utc::now();

        let item = crate::db_fetch_one!(
            pool,
            WebhookOutboxItem,
            r#"
            INSERT INTO webhook_outbox (
                id, webhook_id, owner_id, room_id, severity, title,
                html_body, plain_body, status, attempts, max_attempts,
                next_attempt_at, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'pending', 0, 10, ?, ?)
            RETURNING *;
            "#,
            id,
            webhook_id,
            owner_id,
            room_id,
            severity,
            title,
            html_body,
            plain_body,
            now,
            now
        )?;

        crate::db_execute!(
            pool,
            "UPDATE webhooks SET last_triggered_at = CURRENT_TIMESTAMP WHERE id = ?;",
            webhook_id
        )?;

        Ok(item)
    }

    pub async fn fetch_pending_outbox(
        pool: &DbPool,
        limit: i64,
    ) -> Result<Vec<WebhookOutboxItem>, sqlx::Error> {
        let now = Utc::now();
        crate::db_fetch_all!(
            pool,
            WebhookOutboxItem,
            r#"
            SELECT * FROM webhook_outbox
            WHERE status IN ('pending', 'sending') AND next_attempt_at <= ?
            ORDER BY created_at ASC
            LIMIT ?;
            "#,
            now,
            limit
        )
    }

    pub async fn mark_outbox_delivered(
        pool: &DbPool,
        id: &str,
        webhook_id: &str,
    ) -> Result<(), sqlx::Error> {
        let now = Utc::now();
        crate::db_execute!(
            pool,
            r#"
            UPDATE webhook_outbox
            SET status = 'delivered', delivered_at = ?
            WHERE id = ?;
            "#,
            now,
            id
        )?;

        crate::db_execute!(
            pool,
            "UPDATE webhooks SET total_delivered_count = total_delivered_count + 1 WHERE id = ?;",
            webhook_id
        )?;

        Ok(())
    }

    pub async fn mark_outbox_retry(
        pool: &DbPool,
        id: &str,
        webhook_id: &str,
        attempts: i64,
        next_attempt_at: DateTime<Utc>,
        last_error: &str,
    ) -> Result<(), sqlx::Error> {
        crate::db_execute!(
            pool,
            r#"
            UPDATE webhook_outbox
            SET status = 'pending', attempts = ?, next_attempt_at = ?, last_error = ?
            WHERE id = ?;
            "#,
            attempts,
            next_attempt_at,
            last_error,
            id
        )?;

        crate::db_execute!(
            pool,
            "UPDATE webhooks SET failure_count = failure_count + 1 WHERE id = ?;",
            webhook_id
        )?;

        Ok(())
    }

    pub async fn mark_outbox_failed(
        pool: &DbPool,
        id: &str,
        webhook_id: &str,
        last_error: &str,
    ) -> Result<(), sqlx::Error> {
        crate::db_execute!(
            pool,
            r#"
            UPDATE webhook_outbox
            SET status = 'failed', last_error = ?
            WHERE id = ?;
            "#,
            last_error,
            id
        )?;

        crate::db_execute!(
            pool,
            "UPDATE webhooks SET failure_count = failure_count + 1 WHERE id = ?;",
            webhook_id
        )?;

        Ok(())
    }

    pub async fn list_outbox_by_webhook(
        pool: &DbPool,
        webhook_id: &str,
        owner_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<WebhookOutboxItem>, sqlx::Error> {
        let limit = limit.clamp(1, 100);
        let offset = offset.max(0);
        crate::db_fetch_all!(
            pool,
            WebhookOutboxItem,
            r#"
            SELECT * FROM webhook_outbox
            WHERE webhook_id = ? AND owner_id = ?
            ORDER BY created_at DESC
            LIMIT ? OFFSET ?;
            "#,
            webhook_id,
            owner_id,
            limit,
            offset
        )
    }

    pub async fn count_outbox_by_webhook(
        pool: &DbPool,
        webhook_id: &str,
        owner_id: &str,
    ) -> Result<i64, sqlx::Error> {
        crate::db_scalar_one!(
            pool,
            i64,
            "SELECT COUNT(*) FROM webhook_outbox WHERE webhook_id = ? AND owner_id = ?;",
            webhook_id,
            owner_id
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use sqlx::SqlitePool;

    use crate::accounts::{AccountRepository, STATUS_ACTIVE};

    async fn setup_db() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_webhook_crud_and_outbox() {
        let pool = setup_db().await;
        let acct = AccountRepository::create(&pool, "test", STATUS_ACTIVE)
            .await
            .unwrap();
        let owner_id = &acct;

        let (hook, raw_token) =
            WebhookRepository::create(&pool, owner_id, "VPS Alert", "!room:matrix.org")
                .await
                .unwrap();

        assert_eq!(hook.name, "VPS Alert");
        assert!(raw_token.starts_with("whk_"));

        let found = WebhookRepository::find_by_token_hash(&pool, &hash_token(&raw_token))
            .await
            .unwrap()
            .expect("must find by token hash");
        assert_eq!(found.id, hook.id);

        let outbox = WebhookRepository::enqueue_outbox(
            &pool,
            &hook.id,
            owner_id,
            &hook.default_room_id,
            "critical",
            Some("Server Down"),
            "<b>Server Down</b>",
            "Server Down",
        )
        .await
        .unwrap();

        assert_eq!(outbox.status, "pending");

        let pending = WebhookRepository::fetch_pending_outbox(&pool, 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);

        WebhookRepository::mark_outbox_delivered(&pool, &outbox.id, &hook.id)
            .await
            .unwrap();

        let remaining = WebhookRepository::fetch_pending_outbox(&pool, 10)
            .await
            .unwrap();
        assert_eq!(remaining.len(), 0);
    }
}
