//! Application service for inbound webhooks and the webhook outbox.
//!
//! Orchestrates webhook creation, token rotation, alert enqueueing,
//! outbox draining, and system event publication.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tokio::sync::Notify;

use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::events::{SystemEvent, WebhookAlert};
use crate::domain::webhooks::{
    hash_token, Webhook, WebhookOutboxItem, WebhookSeverity, MAX_NAME_LEN,
};
use crate::events::EventBroadcaster;
use crate::infrastructure::postgres::repositories;

#[derive(Clone)]
pub struct WebhookService {
    pool: PgPool,
    events: EventBroadcaster,
    notify: Arc<Notify>,
}

impl WebhookService {
    pub fn new(pool: PgPool, events: EventBroadcaster) -> Self {
        Self {
            pool,
            events,
            notify: Arc::new(Notify::new()),
        }
    }

    /// Returns the notification signal for waking the outbox worker immediately.
    pub fn get_outbox_notify(&self) -> Arc<Notify> {
        self.notify.clone()
    }

    /// Triggers an immediate wakeup of the background outbox worker.
    pub fn notify_outbox(&self) {
        self.notify.notify_one();
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

        let webhook = repositories::webhooks::create(
            &self.pool,
            &id,
            owner,
            name,
            &token_hash,
            &token_prefix,
            default_room_id,
        )
        .await?;

        self.events
            .send(SystemEvent::WebhookCreated(webhook.clone()));

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

        let updated = repositories::webhooks::update(
            &self.pool,
            id,
            owner,
            name.map(|s| s.trim()),
            default_room_id.map(|s| s.trim()),
            is_active,
        )
        .await?;

        let webhook = updated.ok_or_else(|| AppError::NotFound("Webhook not found".to_string()))?;

        self.events
            .send(SystemEvent::WebhookUpdated(webhook.clone()));

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

        let opt =
            repositories::webhooks::rotate_token(&self.pool, id, owner, &token_hash, &token_prefix)
                .await?;

        let webhook = opt.ok_or_else(|| AppError::NotFound("Webhook not found".to_string()))?;

        self.events
            .send(SystemEvent::WebhookUpdated(webhook.clone()));

        Ok((webhook, raw_token))
    }

    pub async fn delete(&self, owner: &AccountId, id: &str) -> Result<(), AppError> {
        let deleted = repositories::webhooks::delete(&self.pool, id, owner).await?;
        if !deleted {
            return Err(AppError::NotFound("Webhook not found".to_string()));
        }

        self.events.send(SystemEvent::WebhookDeleted {
            id: id.to_string(),
            owner_id: *owner,
        });

        Ok(())
    }

    pub async fn enqueue_alert(
        &self,
        webhook: &Webhook,
        target_room: &str,
        title: Option<&str>,
        html_body: &str,
        plain_body: &str,
        severity: WebhookSeverity,
    ) -> Result<WebhookOutboxItem, AppError> {
        let outbox_id = format!("outbox_{}", uuid::Uuid::new_v4());

        let outbox_item = repositories::webhooks::enqueue_outbox(
            &self.pool,
            &outbox_id,
            &webhook.id,
            &webhook.owner_id,
            target_room,
            severity.as_str(),
            title,
            html_body,
            plain_body,
        )
        .await?;

        self.notify_outbox();

        self.events
            .send(SystemEvent::WebhookTriggered(WebhookAlert {
                id: webhook.id.clone(),
                owner_id: webhook.owner_id,
                room_id: target_room.to_string(),
                severity: severity.as_str().to_string(),
                title: title.map(str::to_string),
                html_body: html_body.to_string(),
                plain_body: plain_body.to_string(),
            }));

        Ok(outbox_item)
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

        let items = repositories::webhooks::list_outbox_by_webhook(
            &self.pool, webhook_id, owner, limit, offset,
        )
        .await?;
        let total =
            repositories::webhooks::count_outbox_by_webhook(&self.pool, webhook_id, owner).await?;

        Ok((items, total))
    }

    pub async fn fetch_pending_outbox(
        &self,
        limit: i64,
    ) -> Result<Vec<WebhookOutboxItem>, AppError> {
        let items = repositories::webhooks::fetch_pending_outbox(&self.pool, limit).await?;
        Ok(items)
    }

    pub async fn mark_outbox_delivered(&self, id: &str, webhook_id: &str) -> Result<(), AppError> {
        repositories::webhooks::mark_outbox_delivered(&self.pool, id, webhook_id).await?;
        Ok(())
    }

    pub async fn mark_outbox_retry(
        &self,
        id: &str,
        webhook_id: &str,
        attempts: i64,
        next_attempt_at: DateTime<Utc>,
        last_error: &str,
    ) -> Result<(), AppError> {
        repositories::webhooks::mark_outbox_retry(
            &self.pool,
            id,
            webhook_id,
            attempts,
            next_attempt_at,
            last_error,
        )
        .await?;
        Ok(())
    }

    pub async fn mark_outbox_failed(
        &self,
        id: &str,
        webhook_id: &str,
        last_error: &str,
    ) -> Result<(), AppError> {
        repositories::webhooks::mark_outbox_failed(&self.pool, id, webhook_id, last_error).await?;
        Ok(())
    }
}
