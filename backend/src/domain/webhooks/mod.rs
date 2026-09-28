//! Domain entities, value objects, and rules for webhooks.

pub mod renderer;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::domain::accounts::AccountId;

pub const MAX_NAME_LEN: usize = 100;
pub const MAX_TITLE_LEN: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Webhook {
    pub id: String,
    pub owner_id: AccountId,
    pub name: String,
    pub token_prefix: String,
    pub default_room_id: String,
    pub is_active: bool,
    pub total_delivered_count: i64,
    pub failure_count: i64,
    pub last_triggered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebhookOutboxItem {
    pub id: String,
    pub webhook_id: String,
    pub owner_id: AccountId,
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

/// Calculates exponential backoff delay based on retry attempts.
pub fn retry_delay(attempts_so_far: i64) -> Duration {
    let exponent = attempts_so_far.clamp(0, 9);
    let base_seconds = 1 << exponent; // 1s, 2s, 4s, 8s, 16s, 32s, 64s, 128s, 256s, 512s
    Duration::seconds(base_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_token_stability() {
        let token = "whk_test_secret_token";
        let h1 = hash_token(token);
        let h2 = hash_token(token);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert_ne!(h1, hash_token("whk_different_token"));
    }

    #[test]
    fn test_retry_delay_exponential_growth_and_cap() {
        assert_eq!(retry_delay(0), Duration::seconds(1));
        assert_eq!(retry_delay(1), Duration::seconds(2));
        assert_eq!(retry_delay(2), Duration::seconds(4));
        assert_eq!(retry_delay(3), Duration::seconds(8));
        assert_eq!(retry_delay(9), Duration::seconds(512));
        // Capped at 2^9 = 512s
        assert_eq!(retry_delay(10), Duration::seconds(512));
        assert_eq!(retry_delay(100), Duration::seconds(512));
    }

    #[test]
    fn test_severity_parsing() {
        assert_eq!(
            WebhookSeverity::from_str_or_default("critical"),
            WebhookSeverity::Critical
        );
        assert_eq!(
            WebhookSeverity::from_str_or_default("error"),
            WebhookSeverity::Critical
        );
        assert_eq!(
            WebhookSeverity::from_str_or_default("warning"),
            WebhookSeverity::Warning
        );
        assert_eq!(
            WebhookSeverity::from_str_or_default("success"),
            WebhookSeverity::Success
        );
        assert_eq!(
            WebhookSeverity::from_str_or_default("default"),
            WebhookSeverity::Default
        );
        assert_eq!(
            WebhookSeverity::from_str_or_default("unknown"),
            WebhookSeverity::Info
        );
    }

    #[test]
    fn test_outbox_status_parsing() {
        assert_eq!(
            OutboxStatus::from_str_or_default("pending"),
            OutboxStatus::Pending
        );
        assert_eq!(
            OutboxStatus::from_str_or_default("sending"),
            OutboxStatus::Sending
        );
        assert_eq!(
            OutboxStatus::from_str_or_default("delivered"),
            OutboxStatus::Delivered
        );
        assert_eq!(
            OutboxStatus::from_str_or_default("failed"),
            OutboxStatus::Failed
        );
        assert_eq!(
            OutboxStatus::from_str_or_default("something"),
            OutboxStatus::Pending
        );
    }
}
