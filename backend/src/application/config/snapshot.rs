//! In-memory snapshot of effective application settings.
//!
//! Provides lock-free, zero-allocation runtime setting lookups with fallback
//! to static defaults defined in `SettingRegistry`.

use std::collections::{HashMap, HashSet};

use crate::domain::config::{SettingKey, SettingRegistry, SettingValue};
use crate::infrastructure::postgres::repositories::app_settings::SettingRow;

/// Immutable snapshot of effective application settings.
#[derive(Debug, Clone)]
pub struct RuntimeConfigSnapshot {
    values: HashMap<SettingKey, SettingValue>,
    customized: HashSet<SettingKey>,
}

impl Default for RuntimeConfigSnapshot {
    fn default() -> Self {
        Self::from_persisted(&[])
    }
}

impl RuntimeConfigSnapshot {
    /// Builds a snapshot from persisted database rows, falling back to registry defaults.
    pub fn from_persisted(rows: &[SettingRow]) -> Self {
        let mut values = HashMap::new();
        let mut customized = HashSet::new();

        // 1. Initialize with all registered defaults
        for def in SettingRegistry::all() {
            values.insert(def.key, def.default_value.clone());
        }

        // 2. Override with persisted rows after validating against registry rules
        for row in rows {
            match row.key.parse::<SettingKey>() {
                Ok(key) => match serde_json::from_value::<SettingValue>(row.value.clone()) {
                    Ok(val) => match SettingRegistry::validate(key, &val) {
                        Ok(()) => {
                            values.insert(key, val);
                            customized.insert(key);
                        }
                        Err(e) => {
                            tracing::warn!(
                                key = %row.key,
                                error = %e,
                                "Ignoring invalid persisted setting value; falling back to default"
                            );
                        }
                    },
                    Err(e) => {
                        tracing::warn!(
                            key = %row.key,
                            error = %e,
                            "Failed to deserialize persisted setting value; falling back to default"
                        );
                    }
                },
                Err(_) => {
                    tracing::warn!(
                        key = %row.key,
                        "Ignoring unknown persisted setting key in app_settings"
                    );
                }
            }
        }

        Self { values, customized }
    }

    /// Returns whether a setting has an explicit database override.
    pub fn is_customized(&self, key: SettingKey) -> bool {
        self.customized.contains(&key)
    }

    /// Returns the count of valid customized settings currently active in the snapshot.
    pub fn customized_count(&self) -> usize {
        self.customized.len()
    }

    /// Fetches the effective value for a setting key.
    pub fn get(&self, key: SettingKey) -> &SettingValue {
        self.values.get(&key).unwrap_or_else(|| {
            static DEFAULT_VAL: SettingValue = SettingValue::Boolean(false);
            &DEFAULT_VAL
        })
    }

    /// Fetches a string setting.
    pub fn get_str(&self, key: SettingKey) -> Option<&str> {
        self.get(key).as_str()
    }

    /// Fetches a boolean setting, defaulting to false if type mismatch.
    pub fn get_bool(&self, key: SettingKey) -> bool {
        self.get(key).as_bool().unwrap_or(false)
    }

    /// Fetches an integer setting, defaulting to 0 if type mismatch.
    pub fn get_i64(&self, key: SettingKey) -> i64 {
        self.get(key).as_i64().unwrap_or(0)
    }

    /// Fetches a float setting, defaulting to 0.0 if type mismatch.
    pub fn get_f64(&self, key: SettingKey) -> f64 {
        self.get(key).as_f64().unwrap_or(0.0)
    }

    // Typed consumer accessors

    pub fn llm_default_provider(&self) -> &str {
        self.get_str(SettingKey::LlmDefaultProvider)
            .unwrap_or("openai")
    }

    pub fn llm_default_model(&self) -> &str {
        self.get_str(SettingKey::LlmDefaultModel)
            .unwrap_or("gpt-4o-mini")
    }

    pub fn llm_request_timeout_seconds(&self) -> i64 {
        self.get_i64(SettingKey::LlmRequestTimeoutSeconds)
    }

    pub fn llm_default_temperature(&self) -> f64 {
        self.get_f64(SettingKey::LlmDefaultTemperature)
    }

    pub fn llm_default_max_tokens(&self) -> i64 {
        self.get_i64(SettingKey::LlmDefaultMaxTokens)
    }

    pub fn search_tavily_enabled(&self) -> bool {
        self.get_bool(SettingKey::SearchTavilyEnabled)
    }

    pub fn search_timeout_seconds(&self) -> i64 {
        self.get_i64(SettingKey::SearchTimeoutSeconds)
    }

    pub fn inference_ocr_enabled(&self) -> bool {
        self.get_bool(SettingKey::InferenceOcrEnabled)
    }

    pub fn inference_ocr_base_url(&self) -> &str {
        self.get_str(SettingKey::InferenceOcrBaseUrl).unwrap_or("")
    }

    pub fn inference_stt_enabled(&self) -> bool {
        self.get_bool(SettingKey::InferenceSttEnabled)
    }

    pub fn inference_stt_base_url(&self) -> &str {
        self.get_str(SettingKey::InferenceSttBaseUrl).unwrap_or("")
    }

    pub fn inference_stt_model(&self) -> &str {
        self.get_str(SettingKey::InferenceSttModel)
            .unwrap_or("sensevoice-small")
    }

    pub fn inference_stt_language(&self) -> &str {
        self.get_str(SettingKey::InferenceSttLanguage)
            .unwrap_or("auto")
    }

    pub fn inference_tts_enabled(&self) -> bool {
        self.get_bool(SettingKey::InferenceTtsEnabled)
    }

    pub fn inference_tts_base_url(&self) -> &str {
        self.get_str(SettingKey::InferenceTtsBaseUrl).unwrap_or("")
    }

    pub fn inference_tts_model(&self) -> &str {
        self.get_str(SettingKey::InferenceTtsModel)
            .unwrap_or("kokoro")
    }

    pub fn inference_tts_voice(&self) -> &str {
        self.get_str(SettingKey::InferenceTtsVoice).unwrap_or("0")
    }

    pub fn inference_tts_dialect(&self) -> &str {
        self.get_str(SettingKey::InferenceTtsDialect)
            .unwrap_or("openai")
    }

    pub fn inference_embedding_base_url(&self) -> &str {
        self.get_str(SettingKey::InferenceEmbeddingBaseUrl)
            .unwrap_or("")
    }

    pub fn registration_allow_link_signup(&self) -> bool {
        self.get_bool(SettingKey::RegistrationAllowLinkSignup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_snapshot_defaults() {
        let snapshot = RuntimeConfigSnapshot::default();
        assert_eq!(snapshot.llm_default_provider(), "openai");
        assert_eq!(snapshot.llm_default_model(), "gpt-4o-mini");
        assert_eq!(snapshot.llm_request_timeout_seconds(), 60);
        assert!(snapshot.search_tavily_enabled());
        assert!(snapshot.registration_allow_link_signup());
    }

    #[test]
    fn test_snapshot_overrides_from_persisted() {
        let rows = vec![
            SettingRow {
                key: "llm.default_model".to_string(),
                value: serde_json::json!("claude-3-5-sonnet"),
                updated_at: Utc::now(),
                updated_by: None,
            },
            SettingRow {
                key: "registration.allow_link_signup".to_string(),
                value: serde_json::json!(false),
                updated_at: Utc::now(),
                updated_by: None,
            },
        ];

        let snapshot = RuntimeConfigSnapshot::from_persisted(&rows);
        assert_eq!(snapshot.llm_default_model(), "claude-3-5-sonnet");
        assert!(!snapshot.registration_allow_link_signup());
        // Defaults preserved for unchanged settings
        assert_eq!(snapshot.llm_default_provider(), "openai");
    }
}
