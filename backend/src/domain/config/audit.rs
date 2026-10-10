//! Dedicated administrative configuration audit domain models.
//!
//! Tracks configuration updates, secret mutations, and resets.
//! Invariant: Audit entries never record plaintext secrets, ciphertexts, or nonces.

use crate::domain::accounts::AccountId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Action recorded in administrative audit logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigAuditAction {
    SettingUpdated,
    SettingReset,
    SecretCreated,
    SecretUpdated,
    SecretDeleted,
    SecretTested,
    ConfigReloaded,
    AdminGranted,
    AdminRevoked,
}

impl ConfigAuditAction {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::SettingUpdated => "setting_updated",
            Self::SettingReset => "setting_reset",
            Self::SecretCreated => "secret_created",
            Self::SecretUpdated => "secret_updated",
            Self::SecretDeleted => "secret_deleted",
            Self::SecretTested => "secret_tested",
            Self::ConfigReloaded => "config_reloaded",
            Self::AdminGranted => "admin_granted",
            Self::AdminRevoked => "admin_revoked",
        }
    }
}

impl std::fmt::Display for ConfigAuditAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ConfigAuditAction {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "setting_updated" => Ok(Self::SettingUpdated),
            "setting_reset" => Ok(Self::SettingReset),
            "secret_created" => Ok(Self::SecretCreated),
            "secret_updated" => Ok(Self::SecretUpdated),
            "secret_deleted" => Ok(Self::SecretDeleted),
            "secret_tested" => Ok(Self::SecretTested),
            "config_reloaded" => Ok(Self::ConfigReloaded),
            "admin_granted" => Ok(Self::AdminGranted),
            "admin_revoked" => Ok(Self::AdminRevoked),
            other => Err(format!("unknown audit action: {other}")),
        }
    }
}

/// Target of an administrative audit action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigAuditTarget {
    Setting(String),
    Secret(uuid::Uuid),
    Account(AccountId),
    System,
}

impl ConfigAuditTarget {
    pub fn target_type(&self) -> &'static str {
        match self {
            Self::Setting(_) => "setting",
            Self::Secret(_) => "secret",
            Self::Account(_) => "account",
            Self::System => "system",
        }
    }

    pub fn target_key(&self) -> String {
        match self {
            Self::Setting(k) => k.clone(),
            Self::Secret(id) => id.to_string(),
            Self::Account(id) => id.to_string(),
            Self::System => "system".to_string(),
        }
    }
}

/// Outcome of the audited action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigAuditOutcome {
    Success,
    Failure,
}

impl ConfigAuditOutcome {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

impl std::fmt::Display for ConfigAuditOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ConfigAuditOutcome {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "success" => Ok(Self::Success),
            "failure" => Ok(Self::Failure),
            other => Err(format!("unknown audit outcome: {other}")),
        }
    }
}

/// Audit log record for administrative configuration changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigAuditEntry {
    pub id: Option<i64>,
    pub actor_id: Option<AccountId>,
    pub action: ConfigAuditAction,
    pub target_type: String,
    pub target_key: String,
    pub outcome: ConfigAuditOutcome,
    pub detail: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

impl ConfigAuditEntry {
    /// Constructs a validated, sanitized audit log entry.
    pub fn new(
        actor_id: Option<AccountId>,
        action: ConfigAuditAction,
        target: ConfigAuditTarget,
        outcome: ConfigAuditOutcome,
        detail: Option<serde_json::Value>,
    ) -> Self {
        let sanitized_detail = detail.map(|d| sanitize_audit_detail(&d));
        Self {
            id: None,
            actor_id,
            action,
            target_type: target.target_type().to_string(),
            target_key: target.target_key(),
            outcome,
            detail: sanitized_detail,
            created_at: Some(Utc::now()),
        }
    }
}

fn is_sensitive_audit_field(k: &str) -> bool {
    let lower = k.to_ascii_lowercase();
    lower == "api_key"
        || lower == "private_key"
        || lower == "master_key"
        || lower == "encryption_key"
        || lower == "recovery_key"
        || lower == "secret_key"
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("token")
        || lower.contains("ciphertext")
        || lower.contains("nonce")
        || lower.contains("credential")
}

/// Strips any accidental secret, ciphertext, nonce, or key payload from audit details.
pub fn sanitize_audit_detail(val: &serde_json::Value) -> serde_json::Value {
    match val {
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (k, v) in map {
                if is_sensitive_audit_field(k) {
                    sanitized.insert(
                        k.clone(),
                        serde_json::Value::String("[REDACTED]".to_string()),
                    );
                } else {
                    sanitized.insert(k.clone(), sanitize_audit_detail(v));
                }
            }
            serde_json::Value::Object(sanitized)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(sanitize_audit_detail).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_action_round_trip() {
        for action in [
            ConfigAuditAction::SettingUpdated,
            ConfigAuditAction::SettingReset,
            ConfigAuditAction::SecretCreated,
            ConfigAuditAction::SecretUpdated,
            ConfigAuditAction::SecretDeleted,
            ConfigAuditAction::SecretTested,
            ConfigAuditAction::ConfigReloaded,
            ConfigAuditAction::AdminGranted,
            ConfigAuditAction::AdminRevoked,
        ] {
            let s = action.as_str();
            let parsed: ConfigAuditAction = s.parse().unwrap();
            assert_eq!(parsed, action);
            assert_eq!(format!("{action}"), s);
        }
    }

    #[test]
    fn test_audit_detail_sanitizes_credentials() {
        let raw = serde_json::json!({
            "provider": "openai",
            "key": "llm.default_model",
            "key_version": 1,
            "api_key": "sk-1234567890abcdef",
            "password": "secret_password",
            "secret_token": "token123",
            "safe_field": "public_info"
        });

        let sanitized = sanitize_audit_detail(&raw);
        assert_eq!(sanitized["provider"], "openai");
        assert_eq!(sanitized["key"], "llm.default_model");
        assert_eq!(sanitized["key_version"], 1);
        assert_eq!(sanitized["safe_field"], "public_info");
        assert_eq!(sanitized["api_key"], "[REDACTED]");
        assert_eq!(sanitized["password"], "[REDACTED]");
        assert_eq!(sanitized["secret_token"], "[REDACTED]");
    }
}
