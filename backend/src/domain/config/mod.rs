//! Pure configuration domain types, invariants, and errors.
//!
//! Owns typed runtime setting keys, values, definitions, and domain validation.
//! Depends only on std, serde, and thiserror.

use serde::{Deserialize, Serialize};

pub mod audit;
pub mod registry;

pub use audit::{ConfigAuditAction, ConfigAuditEntry, ConfigAuditOutcome, ConfigAuditTarget};
pub use registry::{SettingDefinition, SettingRegistry};

/// Strongly typed keys for all recognized runtime application settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SettingKey {
    // LLM
    #[serde(rename = "llm.default_provider")]
    LlmDefaultProvider,
    #[serde(rename = "llm.default_model")]
    LlmDefaultModel,
    #[serde(rename = "llm.request_timeout_seconds")]
    LlmRequestTimeoutSeconds,
    #[serde(rename = "llm.default_temperature")]
    LlmDefaultTemperature,
    #[serde(rename = "llm.default_max_tokens")]
    LlmDefaultMaxTokens,

    // Search
    #[serde(rename = "search.tavily.enabled")]
    SearchTavilyEnabled,
    #[serde(rename = "search.timeout_seconds")]
    SearchTimeoutSeconds,

    // Inference - OCR
    #[serde(rename = "inference.ocr.enabled")]
    InferenceOcrEnabled,
    #[serde(rename = "inference.ocr.base_url")]
    InferenceOcrBaseUrl,

    // Inference - STT
    #[serde(rename = "inference.stt.enabled")]
    InferenceSttEnabled,
    #[serde(rename = "inference.stt.base_url")]
    InferenceSttBaseUrl,
    #[serde(rename = "inference.stt.model")]
    InferenceSttModel,
    #[serde(rename = "inference.stt.language")]
    InferenceSttLanguage,

    // Inference - TTS
    #[serde(rename = "inference.tts.enabled")]
    InferenceTtsEnabled,
    #[serde(rename = "inference.tts.base_url")]
    InferenceTtsBaseUrl,
    #[serde(rename = "inference.tts.model")]
    InferenceTtsModel,
    #[serde(rename = "inference.tts.voice")]
    InferenceTtsVoice,
    #[serde(rename = "inference.tts.dialect")]
    InferenceTtsDialect,

    // Inference - Embedding
    #[serde(rename = "inference.embedding.base_url")]
    InferenceEmbeddingBaseUrl,
    #[serde(rename = "inference.embedding.model")]
    InferenceEmbeddingModel,

    // Channels - Discord
    #[serde(rename = "channels.discord.enabled")]
    ChannelsDiscordEnabled,
    #[serde(rename = "channels.discord.guild_id")]
    ChannelsDiscordGuildId,

    // Channels - Telegram
    #[serde(rename = "channels.telegram.enabled")]
    ChannelsTelegramEnabled,
    #[serde(rename = "channels.telegram.mode")]
    ChannelsTelegramMode,

    // Channels - Email
    #[serde(rename = "channels.email.enabled")]
    ChannelsEmailEnabled,
    #[serde(rename = "channels.email.imap_url")]
    ChannelsEmailImapUrl,
    #[serde(rename = "channels.email.smtp_url")]
    ChannelsEmailSmtpUrl,
    #[serde(rename = "channels.email.from")]
    ChannelsEmailFrom,
    #[serde(rename = "channels.email.authserv_id")]
    ChannelsEmailAuthservId,
    #[serde(rename = "channels.email.poll_seconds")]
    ChannelsEmailPollSeconds,
    #[serde(rename = "channels.email.processed_folder")]
    ChannelsEmailProcessedFolder,

    // Registration
    #[serde(rename = "registration.allow_link_signup")]
    RegistrationAllowLinkSignup,
}

impl SettingKey {
    /// Canonical string identifier for the setting key.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::LlmDefaultProvider => "llm.default_provider",
            Self::LlmDefaultModel => "llm.default_model",
            Self::LlmRequestTimeoutSeconds => "llm.request_timeout_seconds",
            Self::LlmDefaultTemperature => "llm.default_temperature",
            Self::LlmDefaultMaxTokens => "llm.default_max_tokens",
            Self::SearchTavilyEnabled => "search.tavily.enabled",
            Self::SearchTimeoutSeconds => "search.timeout_seconds",
            Self::InferenceOcrEnabled => "inference.ocr.enabled",
            Self::InferenceOcrBaseUrl => "inference.ocr.base_url",
            Self::InferenceSttEnabled => "inference.stt.enabled",
            Self::InferenceSttBaseUrl => "inference.stt.base_url",
            Self::InferenceSttModel => "inference.stt.model",
            Self::InferenceSttLanguage => "inference.stt.language",
            Self::InferenceTtsEnabled => "inference.tts.enabled",
            Self::InferenceTtsBaseUrl => "inference.tts.base_url",
            Self::InferenceTtsModel => "inference.tts.model",
            Self::InferenceTtsVoice => "inference.tts.voice",
            Self::InferenceTtsDialect => "inference.tts.dialect",
            Self::InferenceEmbeddingBaseUrl => "inference.embedding.base_url",
            Self::InferenceEmbeddingModel => "inference.embedding.model",
            Self::ChannelsDiscordEnabled => "channels.discord.enabled",
            Self::ChannelsDiscordGuildId => "channels.discord.guild_id",
            Self::ChannelsTelegramEnabled => "channels.telegram.enabled",
            Self::ChannelsTelegramMode => "channels.telegram.mode",
            Self::ChannelsEmailEnabled => "channels.email.enabled",
            Self::ChannelsEmailImapUrl => "channels.email.imap_url",
            Self::ChannelsEmailSmtpUrl => "channels.email.smtp_url",
            Self::ChannelsEmailFrom => "channels.email.from",
            Self::ChannelsEmailAuthservId => "channels.email.authserv_id",
            Self::ChannelsEmailPollSeconds => "channels.email.poll_seconds",
            Self::ChannelsEmailProcessedFolder => "channels.email.processed_folder",
            Self::RegistrationAllowLinkSignup => "registration.allow_link_signup",
        }
    }

    /// All registered setting keys.
    pub const ALL: &'static [SettingKey] = &[
        Self::LlmDefaultProvider,
        Self::LlmDefaultModel,
        Self::LlmRequestTimeoutSeconds,
        Self::LlmDefaultTemperature,
        Self::LlmDefaultMaxTokens,
        Self::SearchTavilyEnabled,
        Self::SearchTimeoutSeconds,
        Self::InferenceOcrEnabled,
        Self::InferenceOcrBaseUrl,
        Self::InferenceSttEnabled,
        Self::InferenceSttBaseUrl,
        Self::InferenceSttModel,
        Self::InferenceSttLanguage,
        Self::InferenceTtsEnabled,
        Self::InferenceTtsBaseUrl,
        Self::InferenceTtsModel,
        Self::InferenceTtsVoice,
        Self::InferenceTtsDialect,
        Self::InferenceEmbeddingBaseUrl,
        Self::InferenceEmbeddingModel,
        Self::ChannelsDiscordEnabled,
        Self::ChannelsDiscordGuildId,
        Self::ChannelsTelegramEnabled,
        Self::ChannelsTelegramMode,
        Self::ChannelsEmailEnabled,
        Self::ChannelsEmailImapUrl,
        Self::ChannelsEmailSmtpUrl,
        Self::ChannelsEmailFrom,
        Self::ChannelsEmailAuthservId,
        Self::ChannelsEmailPollSeconds,
        Self::ChannelsEmailProcessedFolder,
        Self::RegistrationAllowLinkSignup,
    ];
}

impl std::fmt::Display for SettingKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SettingKey {
    type Err = ConfigError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "llm.default_provider" => Ok(Self::LlmDefaultProvider),
            "llm.default_model" => Ok(Self::LlmDefaultModel),
            "llm.request_timeout_seconds" => Ok(Self::LlmRequestTimeoutSeconds),
            "llm.default_temperature" => Ok(Self::LlmDefaultTemperature),
            "llm.default_max_tokens" => Ok(Self::LlmDefaultMaxTokens),
            "search.tavily.enabled" => Ok(Self::SearchTavilyEnabled),
            "search.timeout_seconds" => Ok(Self::SearchTimeoutSeconds),
            "inference.ocr.enabled" => Ok(Self::InferenceOcrEnabled),
            "inference.ocr.base_url" => Ok(Self::InferenceOcrBaseUrl),
            "inference.stt.enabled" => Ok(Self::InferenceSttEnabled),
            "inference.stt.base_url" => Ok(Self::InferenceSttBaseUrl),
            "inference.stt.model" => Ok(Self::InferenceSttModel),
            "inference.stt.language" => Ok(Self::InferenceSttLanguage),
            "inference.tts.enabled" => Ok(Self::InferenceTtsEnabled),
            "inference.tts.base_url" => Ok(Self::InferenceTtsBaseUrl),
            "inference.tts.model" => Ok(Self::InferenceTtsModel),
            "inference.tts.voice" => Ok(Self::InferenceTtsVoice),
            "inference.tts.dialect" => Ok(Self::InferenceTtsDialect),
            "inference.embedding.base_url" => Ok(Self::InferenceEmbeddingBaseUrl),
            "inference.embedding.model" => Ok(Self::InferenceEmbeddingModel),
            "channels.discord.enabled" => Ok(Self::ChannelsDiscordEnabled),
            "channels.discord.guild_id" => Ok(Self::ChannelsDiscordGuildId),
            "channels.telegram.enabled" => Ok(Self::ChannelsTelegramEnabled),
            "channels.telegram.mode" => Ok(Self::ChannelsTelegramMode),
            "channels.email.enabled" => Ok(Self::ChannelsEmailEnabled),
            "channels.email.imap_url" => Ok(Self::ChannelsEmailImapUrl),
            "channels.email.smtp_url" => Ok(Self::ChannelsEmailSmtpUrl),
            "channels.email.from" => Ok(Self::ChannelsEmailFrom),
            "channels.email.authserv_id" => Ok(Self::ChannelsEmailAuthservId),
            "channels.email.poll_seconds" => Ok(Self::ChannelsEmailPollSeconds),
            "channels.email.processed_folder" => Ok(Self::ChannelsEmailProcessedFolder),
            "registration.allow_link_signup" => Ok(Self::RegistrationAllowLinkSignup),
            unknown => Err(ConfigError::UnknownKey(unknown.to_string())),
        }
    }
}

/// Primitive type classifier for settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingType {
    String,
    Integer,
    Float,
    Boolean,
}

/// Strongly typed runtime setting value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SettingValue {
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
}

impl SettingValue {
    pub fn setting_type(&self) -> SettingType {
        match self {
            Self::Boolean(_) => SettingType::Boolean,
            Self::Integer(_) => SettingType::Integer,
            Self::Float(_) => SettingType::Float,
            Self::String(_) => SettingType::String,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(f) => Some(*f),
            Self::Integer(i) => Some(*i as f64),
            _ => None,
        }
    }
}

impl From<bool> for SettingValue {
    fn from(b: bool) -> Self {
        Self::Boolean(b)
    }
}

impl From<i64> for SettingValue {
    fn from(i: i64) -> Self {
        Self::Integer(i)
    }
}

impl From<f64> for SettingValue {
    fn from(f: f64) -> Self {
        Self::Float(f)
    }
}

impl From<String> for SettingValue {
    fn from(s: String) -> Self {
        Self::String(s)
    }
}

impl From<&str> for SettingValue {
    fn from(s: &str) -> Self {
        Self::String(s.to_string())
    }
}

/// Errors raised by domain configuration operations.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    #[error("Unknown setting key: {0}")]
    UnknownKey(String),

    #[error("Type mismatch for {key}: expected {expected:?}, got {actual:?}")]
    TypeMismatch {
        key: String,
        expected: SettingType,
        actual: SettingType,
    },

    #[error("Validation failed for {key}: {reason}")]
    Validation { key: String, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setting_key_string_round_trip() {
        for &key in SettingKey::ALL {
            let s = key.as_str();
            let parsed: SettingKey = s.parse().expect("key should parse");
            assert_eq!(parsed, key);
            assert_eq!(format!("{key}"), s);
        }
    }

    #[test]
    fn test_unknown_setting_key_rejected() {
        let res = "unknown.setting.key".parse::<SettingKey>();
        assert_eq!(
            res,
            Err(ConfigError::UnknownKey("unknown.setting.key".to_string()))
        );
    }

    #[test]
    fn test_setting_value_conversions() {
        let b = SettingValue::Boolean(true);
        assert_eq!(b.as_bool(), Some(true));
        assert_eq!(b.setting_type(), SettingType::Boolean);

        let i = SettingValue::Integer(42);
        assert_eq!(i.as_i64(), Some(42));
        assert_eq!(i.as_f64(), Some(42.0));
        assert_eq!(i.setting_type(), SettingType::Integer);

        let f = SettingValue::Float(1.25);
        assert_eq!(f.as_f64(), Some(1.25));
        assert_eq!(f.setting_type(), SettingType::Float);

        let s = SettingValue::String("hello".to_string());
        assert_eq!(s.as_str(), Some("hello"));
        assert_eq!(s.setting_type(), SettingType::String);
    }

    #[test]
    fn test_registry_validation_bounds() {
        // LLM temperature bounds [0.0, 2.0]
        assert!(SettingRegistry::validate(
            SettingKey::LlmDefaultTemperature,
            &SettingValue::Float(0.7)
        )
        .is_ok());
        assert!(SettingRegistry::validate(
            SettingKey::LlmDefaultTemperature,
            &SettingValue::Float(2.5)
        )
        .is_err());

        // LLM timeout bounds [5, 300]
        assert!(SettingRegistry::validate(
            SettingKey::LlmRequestTimeoutSeconds,
            &SettingValue::Integer(60)
        )
        .is_ok());
        assert!(SettingRegistry::validate(
            SettingKey::LlmRequestTimeoutSeconds,
            &SettingValue::Integer(2)
        )
        .is_err());

        // LLM provider validation
        assert!(SettingRegistry::validate(
            SettingKey::LlmDefaultProvider,
            &SettingValue::String("openai".to_string())
        )
        .is_ok());
        assert!(SettingRegistry::validate(
            SettingKey::LlmDefaultProvider,
            &SettingValue::String("unsupported_vendor".to_string())
        )
        .is_err());

        // Type mismatch
        assert!(matches!(
            SettingRegistry::validate(
                SettingKey::SearchTavilyEnabled,
                &SettingValue::String("not_a_bool".to_string())
            ),
            Err(ConfigError::TypeMismatch { .. })
        ));
    }
}
