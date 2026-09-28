//! Data Transfer Objects and schemas for webhooks.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WebhookSeverity {
    Critical,
    Warning,
    Info,
    Success,
    Default,
}

impl From<crate::domain::webhooks::WebhookSeverity> for WebhookSeverity {
    fn from(s: crate::domain::webhooks::WebhookSeverity) -> Self {
        match s {
            crate::domain::webhooks::WebhookSeverity::Critical => Self::Critical,
            crate::domain::webhooks::WebhookSeverity::Warning => Self::Warning,
            crate::domain::webhooks::WebhookSeverity::Info => Self::Info,
            crate::domain::webhooks::WebhookSeverity::Success => Self::Success,
            crate::domain::webhooks::WebhookSeverity::Default => Self::Default,
        }
    }
}

impl From<WebhookSeverity> for crate::domain::webhooks::WebhookSeverity {
    fn from(s: WebhookSeverity) -> Self {
        match s {
            WebhookSeverity::Critical => Self::Critical,
            WebhookSeverity::Warning => Self::Warning,
            WebhookSeverity::Info => Self::Info,
            WebhookSeverity::Success => Self::Success,
            WebhookSeverity::Default => Self::Default,
        }
    }
}

impl WebhookSeverity {
    pub fn from_str_or_default(s: &str) -> Self {
        crate::domain::webhooks::WebhookSeverity::from_str_or_default(s).into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum OutboxStatus {
    Pending,
    Sending,
    Delivered,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Webhook {
    pub id: String,
    pub owner_id: String,
    pub name: String,
    pub token_prefix: String,
    pub default_room_id: String,
    pub is_active: bool,
    pub total_delivered_count: i64,
    pub failure_count: i64,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<crate::domain::webhooks::Webhook> for Webhook {
    fn from(w: crate::domain::webhooks::Webhook) -> Self {
        Self {
            id: w.id,
            owner_id: w.owner_id.to_string(),
            name: w.name,
            token_prefix: w.token_prefix,
            default_room_id: w.default_room_id,
            is_active: w.is_active,
            total_delivered_count: w.total_delivered_count,
            failure_count: w.failure_count,
            last_triggered_at: w.last_triggered_at,
            created_at: w.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookOutboxItem {
    pub id: String,
    pub webhook_id: String,
    pub owner_id: String,
    pub room_id: String,
    pub severity: String,
    pub title: Option<String>,
    pub html_body: String,
    pub plain_body: String,
    pub status: String,
    pub attempts: i64,
    pub max_attempts: i64,
    pub next_attempt_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub delivered_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::webhooks::WebhookOutboxItem> for WebhookOutboxItem {
    fn from(item: crate::domain::webhooks::WebhookOutboxItem) -> Self {
        Self {
            id: item.id,
            webhook_id: item.webhook_id,
            owner_id: item.owner_id.to_string(),
            room_id: item.room_id,
            severity: item.severity,
            title: item.title,
            html_body: item.html_body,
            plain_body: item.plain_body,
            status: item.status,
            attempts: item.attempts,
            max_attempts: item.max_attempts,
            next_attempt_at: item.next_attempt_at,
            last_error: item.last_error,
            created_at: item.created_at,
            delivered_at: item.delivered_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreatedWebhookResponse {
    pub webhook: Webhook,
    pub raw_token: String,
    pub example_curl_json: String,
    pub example_curl_telegram: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateWebhookInput {
    pub name: String,
    pub default_room_id: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateWebhookInput {
    pub name: Option<String>,
    pub default_room_id: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InboundPayload {
    pub title: Option<String>,
    pub message: Option<String>,
    pub text: Option<String>,
    pub level: Option<String>,
    pub room_id: Option<String>,
    pub chat_id: Option<String>,
    pub format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, IntoParams, ToSchema)]
pub struct TelegramQuery {
    pub chat_id: Option<String>,
    pub text: Option<String>,
    pub parse_mode: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InboundAcceptedResponse {
    pub status: &'static str,
    pub outbox_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelegramResponse {
    pub ok: bool,
    pub result: TelegramMessageResult,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TelegramMessageResult {
    pub message_id: i64,
    pub date: i64,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct WebhookLogsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookLogsResponse {
    pub items: Vec<WebhookOutboxItem>,
    pub total: i64,
    pub limit: i64,
    pub offset: i64,
}

pub fn build_example_curls(raw_token: &str) -> (String, String) {
    let json = format!(
        r#"curl -X POST https://<host>/api/v1/webhooks/{raw_token} -H "Content-Type: application/json" -d '{{"title":"Alert","message":"Server is down","level":"critical"}}'"#
    );
    let tg =
        format!(r#"curl -X POST https://<host>/bot{raw_token}/sendMessage -d "text=Server alert""#);
    (json, tg)
}
