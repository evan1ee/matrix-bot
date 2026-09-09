use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum WebhookSeverity {
    Critical,
    Warning,
    Info,
    Success,
    Default,
}

impl WebhookSeverity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::Warning => "warning",
            Self::Info => "info",
            Self::Success => "success",
            Self::Default => "default",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "critical" | "error" | "fatal" | "alert" | "danger" => Self::Critical,
            "warning" | "warn" => Self::Warning,
            "success" | "ok" => Self::Success,
            "default" | "normal" => Self::Default,
            _ => Self::Info,
        }
    }

    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Critical => "🔴",
            Self::Warning => "🟡",
            Self::Info => "ℹ️",
            Self::Success => "🟢",
            Self::Default => "🔔",
        }
    }

    pub fn color_hex(&self) -> &'static str {
        match self {
            Self::Critical => "#e5484d",
            Self::Warning => "#f5a623",
            Self::Info => "#3b82f6",
            Self::Success => "#30a46c",
            Self::Default => "#8b8d98",
        }
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

impl OutboxStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sending => "sending",
            Self::Delivered => "delivered",
            Self::Failed => "failed",
        }
    }

    pub fn from_str_or_default(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "sending" => Self::Sending,
            "delivered" => Self::Delivered,
            "failed" => Self::Failed,
            _ => Self::Pending,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Serialize)]
pub struct TelegramResponse {
    pub ok: bool,
    pub result: TelegramMessageResult,
}

#[derive(Debug, Serialize)]
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

