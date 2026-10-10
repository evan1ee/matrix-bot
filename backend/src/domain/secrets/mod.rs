//! Pure secrets domain types, metadata, redaction, and invariants.
//!
//! Encrypted secrets are stored at rest and strictly write-only in read APIs.
//! Depends only on std, chrono, uuid, serde, and thiserror.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Strongly typed identifier for a stored secret credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SecretId(uuid::Uuid);

impl SecretId {
    pub fn new(id: impl Into<SecretId>) -> Self {
        id.into()
    }

    pub fn from_uuid(u: uuid::Uuid) -> Self {
        Self(u)
    }

    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub fn uuid(&self) -> uuid::Uuid {
        self.0
    }

    pub fn as_uuid(&self) -> &uuid::Uuid {
        &self.0
    }
}

impl std::fmt::Display for SecretId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for SecretId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        uuid::Uuid::parse_str(s).map(Self)
    }
}

impl From<uuid::Uuid> for SecretId {
    fn from(u: uuid::Uuid) -> Self {
        Self(u)
    }
}

impl From<SecretId> for uuid::Uuid {
    fn from(id: SecretId) -> Self {
        id.0
    }
}

impl std::ops::Deref for SecretId {
    type Target = uuid::Uuid;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Provider category for an external integration credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretProvider {
    OpenAi,
    Anthropic,
    Tavily,
    Stt,
    Tts,
    Discord,
    Telegram,
    TelegramWebhook,
    Email,
    S3,
    Generic,
}

impl SecretProvider {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::OpenAi => "openai",
            Self::Anthropic => "anthropic",
            Self::Tavily => "tavily",
            Self::Stt => "stt",
            Self::Tts => "tts",
            Self::Discord => "discord",
            Self::Telegram => "telegram",
            Self::TelegramWebhook => "telegram_webhook",
            Self::Email => "email",
            Self::S3 => "s3",
            Self::Generic => "generic",
        }
    }
}

impl std::fmt::Display for SecretProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SecretProvider {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "openai" | "ai_default" => Ok(Self::OpenAi),
            "anthropic" => Ok(Self::Anthropic),
            "tavily" => Ok(Self::Tavily),
            "stt" => Ok(Self::Stt),
            "tts" => Ok(Self::Tts),
            "discord" => Ok(Self::Discord),
            "telegram" => Ok(Self::Telegram),
            "telegram_webhook" => Ok(Self::TelegramWebhook),
            "email" => Ok(Self::Email),
            "s3" => Ok(Self::S3),
            "generic" => Ok(Self::Generic),
            other => Err(SecretError::InvalidProvider(other.to_string())),
        }
    }
}

/// Lifecycle status for a stored credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretStatus {
    Active,
    Disabled,
    Revoked,
}

impl SecretStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Revoked => "revoked",
        }
    }
}

impl std::fmt::Display for SecretStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SecretStatus {
    type Err = SecretError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "disabled" => Ok(Self::Disabled),
            "revoked" => Ok(Self::Revoked),
            other => Err(SecretError::InvalidStatus(other.to_string())),
        }
    }
}

/// Non-sensitive public metadata representation of a stored secret.
/// Guarantees that neither plaintext secret nor ciphertext is exposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretMetadata {
    pub id: SecretId,
    pub provider: SecretProvider,
    pub name: String,
    pub status: SecretStatus,
    pub key_version: i32,
    pub masked_preview: Option<String>,
    pub last_tested_at: Option<DateTime<Utc>>,
    pub last_test_status: Option<String>,
    pub last_test_error: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub metadata: Option<serde_json::Value>,
}

/// Encrypted ciphertext and nonce payload as stored in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSecret {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
    pub key_version: i32,
}

/// In-memory wrapper for plaintext secrets with memory zeroization on drop
/// and redacted Debug/Display representations.
pub struct SecretValue(String);

impl SecretValue {
    /// Creates a new secret value wrapper.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// Explicit opt-in to expose plaintext secret to authorized outbound clients.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl std::fmt::Display for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[REDACTED]")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            unsafe {
                let vec = self.0.as_mut_vec();
                std::ptr::write_bytes(vec.as_mut_ptr(), 0, vec.len());
            }
        }
    }
}

/// Generates a masked preview for UI presentation (e.g., `sk-...4a1b` or `••••...4a1b`).
pub fn mask_secret(secret: &str) -> String {
    let trimmed = secret.trim();
    if trimmed.len() <= 6 {
        "••••••••".to_string()
    } else if trimmed.starts_with("sk-") && trimmed.len() > 10 {
        let end = &trimmed[trimmed.len() - 4..];
        format!("sk-...{end}")
    } else {
        let end = &trimmed[trimmed.len() - 4..];
        format!("••••...{end}")
    }
}

/// Errors raised in secrets domain operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SecretError {
    #[error("Invalid secret provider: {0}")]
    InvalidProvider(String),

    #[error("Invalid secret status: {0}")]
    InvalidStatus(String),

    #[error("Secret empty or payload too large")]
    InvalidPayload,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_secret_id_round_trip() {
        let id = SecretId::generate();
        let s = id.to_string();
        let parsed: SecretId = s.parse().unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn test_secret_provider_round_trip() {
        for provider in [
            SecretProvider::OpenAi,
            SecretProvider::Anthropic,
            SecretProvider::Tavily,
            SecretProvider::Stt,
            SecretProvider::Tts,
            SecretProvider::Discord,
            SecretProvider::Telegram,
            SecretProvider::TelegramWebhook,
            SecretProvider::Email,
            SecretProvider::S3,
            SecretProvider::Generic,
        ] {
            let s = provider.as_str();
            let parsed: SecretProvider = s.parse().unwrap();
            assert_eq!(parsed, provider);
            assert_eq!(format!("{provider}"), s);
        }
    }

    #[test]
    fn test_secret_value_redaction_and_exposure() {
        let val = SecretValue::new("sk-proj-supersecret1234");
        assert_eq!(format!("{val}"), "[REDACTED]");
        assert_eq!(format!("{val:?}"), "[REDACTED]");
        assert_eq!(val.expose_secret(), "sk-proj-supersecret1234");
    }

    #[test]
    fn test_mask_secret_previews() {
        assert_eq!(mask_secret("sk-1234567890abcdef"), "sk-...cdef");
        assert_eq!(mask_secret("ghp_myfaketoken1234"), "••••...1234");
        assert_eq!(mask_secret("short"), "••••••••");
    }
}
