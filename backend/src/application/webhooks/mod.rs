//! Application service for inbound webhooks.
//!
//! Orchestrates webhook creation, token rotation and alert enqueueing. An
//! accepted alert is one `WebhookTriggered` outbox event; its delivery and
//! its logs are read from the outbox (`application/outbox`).

use sqlx::PgPool;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::{DeliveryId, SystemEvent, WebhookAlert};
use crate::domain::webhooks::{
    hash_token, Webhook, WebhookOutboxItem, WebhookSeverity, MAX_NAME_LEN,
};
use crate::infrastructure::postgres::repositories::{self, outbox};

#[derive(Clone)]
pub struct WebhookService {
    pool: PgPool,
}

impl WebhookService {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        owner: &AccountId,
        name: &str,
        default_room_id: &str,
    ) -> Result<(Webhook, String), AppError> {
        let name = name.trim();
        if name.is_empty() || name.len() > MAX_NAME_LEN {
            return Err(AppError::Validation(
                "Webhook name must be between 1 and 100 characters".to_string(),
            ));
        }

        let default_room_id = default_room_id.trim();
        if default_room_id.is_empty() {
            return Err(AppError::Validation(
                "Target room ID cannot be empty".to_string(),
            ));
        }

        let raw_random = crate::utils::random::random_hex(24);
        let raw_token = format!("whk_{raw_random}");
        let token_hash = hash_token(&raw_token);
        let token_prefix = raw_token.chars().take(10).collect::<String>();
        let id = format!("whk_{}", uuid::Uuid::new_v4());

        let mut tx = self.pool.begin().await?;
        let webhook = repositories::webhooks::create(
            &mut *tx,
            &id,
            owner,
            name,
            &token_hash,
            &token_prefix,
            default_room_id,
        )
        .await?;

        outbox::publish(&mut tx, &SystemEvent::WebhookCreated(webhook.clone())).await?;
        tx.commit().await?;

        Ok((webhook, raw_token))
    }

    pub async fn get(&self, owner: &AccountId, id: &str) -> Result<Webhook, AppError> {
        let opt = repositories::webhooks::find_by_id(&self.pool, id, owner).await?;
        opt.ok_or_else(|| AppError::NotFound("Webhook not found".to_string()))
    }

    pub async fn find_by_token_hash(&self, token_hash: &str) -> Result<Option<Webhook>, AppError> {
        let opt = repositories::webhooks::find_by_token_hash(&self.pool, token_hash).await?;
        Ok(opt)
    }

    pub async fn list(&self, owner: &AccountId) -> Result<Vec<Webhook>, AppError> {
        let items = repositories::webhooks::list_by_owner(&self.pool, owner).await?;
        Ok(items)
    }

    pub async fn update(
        &self,
        owner: &AccountId,
        id: &str,
        name: Option<&str>,
        default_room_id: Option<&str>,
        is_active: Option<bool>,
    ) -> Result<Webhook, AppError> {
        if let Some(n) = name {
            let trimmed = n.trim();
            if trimmed.is_empty() || trimmed.len() > MAX_NAME_LEN {
                return Err(AppError::Validation(
                    "Webhook name must be between 1 and 100 characters".to_string(),
                ));
            }
        }

        let mut tx = self.pool.begin().await?;
        let updated = repositories::webhooks::update(
            &mut *tx,
            id,
            owner,
            name.map(|s| s.trim()),
            default_room_id.map(|s| s.trim()),
            is_active,
        )
        .await?;

        let webhook = updated.ok_or_else(|| AppError::NotFound("Webhook not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::WebhookUpdated(webhook.clone())).await?;
        tx.commit().await?;

        Ok(webhook)
    }

    pub async fn rotate_token(
        &self,
        owner: &AccountId,
        id: &str,
    ) -> Result<(Webhook, String), AppError> {
        let raw_random = crate::utils::random::random_hex(24);
        let raw_token = format!("whk_{raw_random}");
        let token_hash = hash_token(&raw_token);
        let token_prefix = raw_token.chars().take(10).collect::<String>();

        let mut tx = self.pool.begin().await?;
        let opt =
            repositories::webhooks::rotate_token(&mut *tx, id, owner, &token_hash, &token_prefix)
                .await?;

        let webhook = opt.ok_or_else(|| AppError::NotFound("Webhook not found".to_string()))?;

        outbox::publish(&mut tx, &SystemEvent::WebhookUpdated(webhook.clone())).await?;
        tx.commit().await?;

        Ok((webhook, raw_token))
    }

    pub async fn delete(&self, owner: &AccountId, id: &str) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let deleted = repositories::webhooks::delete(&mut *tx, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound("Webhook not found".to_string()));
        }

        outbox::publish(
            &mut tx,
            &SystemEvent::WebhookDeleted {
                id: id.to_string(),
                owner_id: *owner,
            },
        )
        .await?;
        tx.commit().await?;

        Ok(())
    }

    /// Accepts one alert: publishes it and stamps the webhook in one
    /// transaction, and returns the id of its Matrix delivery.
    pub async fn enqueue_alert(
        &self,
        webhook: &Webhook,
        target_room: &str,
        title: Option<&str>,
        html_body: &str,
        plain_body: &str,
        severity: WebhookSeverity,
    ) -> Result<DeliveryId, AppError> {
        let alert = WebhookAlert {
            id: webhook.id.clone(),
            owner_id: webhook.owner_id,
            room_id: target_room.to_string(),
            severity: severity.as_str().to_string(),
            title: title.map(str::to_string),
            html_body: html_body.to_string(),
            plain_body: plain_body.to_string(),
        };

        let mut tx = self.pool.begin().await?;
        let published = outbox::publish(&mut tx, &SystemEvent::WebhookTriggered(alert)).await?;
        repositories::webhooks::touch_last_triggered(&mut *tx, &webhook.id).await?;
        tx.commit().await?;

        // `WebhookTriggered` has exactly one destination; an empty list is a
        // broken invariant. `Database` is the opaque-500 variant whose detail
        // is logged and never shown to a client.
        published.deliveries.first().copied().ok_or_else(|| {
            AppError::Database(sqlx::Error::Protocol(
                "WebhookTriggered was published without a delivery".to_string(),
            ))
        })
    }

    pub async fn list_logs(
        &self,
        owner: &AccountId,
        webhook_id: &str,
        limit: i64,
        offset: i64,
    ) -> Result<(Vec<WebhookOutboxItem>, i64), AppError> {
        // Enforce ownership of the webhook before querying its logs
        let _ = self.get(owner, webhook_id).await?;

        let items = outbox::list_webhook_logs(&self.pool, owner, webhook_id, limit, offset).await?;
        let total = outbox::count_webhook_logs(&self.pool, owner, webhook_id).await?;

        Ok((items, total))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn services(seed: &str) -> (crate::application::AppServices, AccountId) {
        let pool = crate::db::pool::test_pool_sized(4).await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::from_seed(seed);
        crate::db::pool::seed_account(&pool, &owner).await;
        (crate::application::AppServices::new(&pool), owner)
    }

    #[tokio::test]
    async fn list_logs_reads_an_outbox_delivery_as_a_log_row() {
        let (services, owner) = services("acct_logs_map").await;
        let (webhook, _) = services
            .webhooks
            .create(&owner, "ci", "!room:example.org")
            .await
            .unwrap();
        let id = services
            .webhooks
            .enqueue_alert(
                &webhook,
                "!room:example.org",
                Some("Disk"),
                "<b>full</b>",
                "full",
                WebhookSeverity::Critical,
            )
            .await
            .unwrap();

        let (logs, total) = services
            .webhooks
            .list_logs(&owner, &webhook.id, 10, 0)
            .await
            .unwrap();
        assert_eq!(total, 1);
        let log = &logs[0];
        assert_eq!(log.id, id.to_string());
        assert_eq!(log.webhook_id, webhook.id);
        assert_eq!(log.owner_id, owner);
        assert_eq!(log.room_id, "!room:example.org");
        assert_eq!(log.title.as_deref(), Some("Disk"));
        assert_eq!(log.html_body, "<b>full</b>");
        assert_eq!(log.plain_body, "full");
        assert_eq!(log.severity, "critical");
        assert_eq!(log.status, "pending");
        assert_eq!(log.attempts, 0);
        assert_eq!(log.max_attempts, 10);
        let stamped = services.webhooks.get(&owner, &webhook.id).await.unwrap();
        assert!(
            stamped.last_triggered_at.is_some(),
            "C1: the enqueue stamps it"
        );
    }

    #[tokio::test]
    async fn another_accounts_logs_are_not_visible() {
        let (services, owner) = services("acct_logs_owner").await;
        let other = AccountId::from_seed("acct_logs_other");
        crate::db::pool::seed_account(&services.outbox.pool().clone(), &other).await;
        let (webhook, _) = services
            .webhooks
            .create(&owner, "ci", "!room:example.org")
            .await
            .unwrap();
        services
            .webhooks
            .enqueue_alert(
                &webhook,
                "!room:example.org",
                None,
                "h",
                "h",
                WebhookSeverity::Info,
            )
            .await
            .unwrap();

        assert!(matches!(
            services
                .webhooks
                .list_logs(&other, &webhook.id, 10, 0)
                .await,
            Err(AppError::NotFound(_))
        ));
    }
}
