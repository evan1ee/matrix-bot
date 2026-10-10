//! Setting definitions catalog and domain validation rules.

use super::{ConfigError, SettingKey, SettingType, SettingValue};

/// Defines schema, default value, reload capability, and validation rules for a setting.
#[derive(Debug, Clone)]
pub struct SettingDefinition {
    pub key: SettingKey,
    pub value_type: SettingType,
    pub default_value: SettingValue,
    pub hot_reloadable: bool,
    pub description: &'static str,
    pub validator: fn(&SettingKey, &SettingValue) -> Result<(), ConfigError>,
}

fn validate_string_max_500(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        if s.len() > 500 {
            return Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: "value exceeds maximum length of 500 characters".to_string(),
            });
        }
        Ok(())
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_non_empty_string(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: "value cannot be empty".to_string(),
            });
        }
        if trimmed.len() > 500 {
            return Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: "value exceeds maximum length of 500 characters".to_string(),
            });
        }
        Ok(())
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_optional_url(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if !(trimmed.starts_with("http://")
            || trimmed.starts_with("https://")
            || trimmed.starts_with("imaps://")
            || trimmed.starts_with("smtps://"))
        {
            return Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: "URL must start with http://, https://, imaps://, or smtps://".to_string(),
            });
        }
        if trimmed.len() > 500 {
            return Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: "URL exceeds maximum length of 500 characters".to_string(),
            });
        }
        Ok(())
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_optional_email(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Ok(());
        }
        if !trimmed.contains('@') || trimmed.len() > 500 {
            return Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: "invalid email address format".to_string(),
            });
        }
        Ok(())
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_llm_provider(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        match s.to_ascii_lowercase().as_str() {
            "openai" | "anthropic" | "xai" | "deepseek" | "ollama" | "custom" => Ok(()),
            other => Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!(
                    "unsupported provider '{other}'; must be one of openai, anthropic, xai, deepseek, ollama, custom"
                ),
            }),
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_llm_timeout(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(i) = val.as_i64() {
        if (5..=300).contains(&i) {
            Ok(())
        } else {
            Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("timeout {i} out of allowed range [5, 300] seconds"),
            })
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::Integer,
            actual: val.setting_type(),
        })
    }
}

fn validate_llm_temperature(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(f) = val.as_f64() {
        if (0.0..=2.0).contains(&f) {
            Ok(())
        } else {
            Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("temperature {f} out of allowed range [0.0, 2.0]"),
            })
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::Float,
            actual: val.setting_type(),
        })
    }
}

fn validate_llm_max_tokens(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(i) = val.as_i64() {
        if (64..=8192).contains(&i) {
            Ok(())
        } else {
            Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("max tokens {i} out of allowed range [64, 8192]"),
            })
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::Integer,
            actual: val.setting_type(),
        })
    }
}

fn validate_search_timeout(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(i) = val.as_i64() {
        if (3..=60).contains(&i) {
            Ok(())
        } else {
            Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("search timeout {i} out of allowed range [3, 60] seconds"),
            })
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::Integer,
            actual: val.setting_type(),
        })
    }
}

fn validate_boolean(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if val.as_bool().is_some() {
        Ok(())
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::Boolean,
            actual: val.setting_type(),
        })
    }
}

fn validate_tts_dialect(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        match s.to_ascii_lowercase().as_str() {
            "openai" | "elevenlabs" => Ok(()),
            other => Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("unsupported TTS dialect '{other}'; must be openai or elevenlabs"),
            }),
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_telegram_mode(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(s) = val.as_str() {
        match s.to_ascii_lowercase().as_str() {
            "polling" | "webhook" => Ok(()),
            other => Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("unsupported telegram mode '{other}'; must be polling or webhook"),
            }),
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::String,
            actual: val.setting_type(),
        })
    }
}

fn validate_email_poll_seconds(key: &SettingKey, val: &SettingValue) -> Result<(), ConfigError> {
    if let Some(i) = val.as_i64() {
        if (5..=86400).contains(&i) {
            Ok(())
        } else {
            Err(ConfigError::Validation {
                key: key.as_str().to_string(),
                reason: format!("poll seconds {i} must be at least 5 seconds"),
            })
        }
    } else {
        Err(ConfigError::TypeMismatch {
            key: key.as_str().to_string(),
            expected: SettingType::Integer,
            actual: val.setting_type(),
        })
    }
}

/// Catalog of definitions for all supported settings.
pub struct SettingRegistry;

impl SettingRegistry {
    /// Retrieves definition for a typed setting key.
    pub fn definition(key: SettingKey) -> &'static SettingDefinition {
        static DEFINITIONS: std::sync::OnceLock<Vec<SettingDefinition>> =
            std::sync::OnceLock::new();
        let defs = DEFINITIONS.get_or_init(Self::build_definitions);
        let idx = key as usize;
        &defs[idx]
    }

    /// Resolves definition by string identifier.
    pub fn get(name: &str) -> Result<&'static SettingDefinition, ConfigError> {
        let key: SettingKey = name.parse()?;
        Ok(Self::definition(key))
    }

    /// Validates setting value against its registered definition and rules.
    pub fn validate(key: SettingKey, value: &SettingValue) -> Result<(), ConfigError> {
        let def = Self::definition(key);
        (def.validator)(&key, value)
    }

    /// Returns all registered setting definitions.
    pub fn all() -> &'static [SettingDefinition] {
        static DEFINITIONS: std::sync::OnceLock<Vec<SettingDefinition>> =
            std::sync::OnceLock::new();
        DEFINITIONS.get_or_init(Self::build_definitions).as_slice()
    }

    fn build_definitions() -> Vec<SettingDefinition> {
        vec![
            SettingDefinition {
                key: SettingKey::LlmDefaultProvider,
                value_type: SettingType::String,
                default_value: SettingValue::String("openai".to_string()),
                hot_reloadable: true,
                description: "Active default LLM provider integration (openai, anthropic, xai, deepseek, ollama, custom)",
                validator: validate_llm_provider,
            },
            SettingDefinition {
                key: SettingKey::LlmDefaultModel,
                value_type: SettingType::String,
                default_value: SettingValue::String("gpt-4o-mini".to_string()),
                hot_reloadable: true,
                description: "Default model name passed in inference requests",
                validator: validate_non_empty_string,
            },
            SettingDefinition {
                key: SettingKey::LlmRequestTimeoutSeconds,
                value_type: SettingType::Integer,
                default_value: SettingValue::Integer(60),
                hot_reloadable: true,
                description: "HTTP client timeout for LLM calls (5–300 seconds)",
                validator: validate_llm_timeout,
            },
            SettingDefinition {
                key: SettingKey::LlmDefaultTemperature,
                value_type: SettingType::Float,
                default_value: SettingValue::Float(0.7),
                hot_reloadable: true,
                description: "Default sampling temperature (0.0–2.0)",
                validator: validate_llm_temperature,
            },
            SettingDefinition {
                key: SettingKey::LlmDefaultMaxTokens,
                value_type: SettingType::Integer,
                default_value: SettingValue::Integer(2048),
                hot_reloadable: true,
                description: "Max completion tokens (64–8192)",
                validator: validate_llm_max_tokens,
            },
            SettingDefinition {
                key: SettingKey::SearchTavilyEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(true),
                hot_reloadable: true,
                description: "Whether Tavily web search is used when configured",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::SearchTimeoutSeconds,
                value_type: SettingType::Integer,
                default_value: SettingValue::Integer(15),
                hot_reloadable: true,
                description: "Web search request timeout (3–60 seconds)",
                validator: validate_search_timeout,
            },
            SettingDefinition {
                key: SettingKey::InferenceOcrEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(true),
                hot_reloadable: true,
                description: "Enable/disable OCR image text recognition",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::InferenceOcrBaseUrl,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: true,
                description: "Base URL for OCR inference service",
                validator: validate_optional_url,
            },
            SettingDefinition {
                key: SettingKey::InferenceSttEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(true),
                hot_reloadable: true,
                description: "Enable/disable speech-to-text audio transcription",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::InferenceSttBaseUrl,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: true,
                description: "Base URL for STT inference service",
                validator: validate_optional_url,
            },
            SettingDefinition {
                key: SettingKey::InferenceSttModel,
                value_type: SettingType::String,
                default_value: SettingValue::String("sensevoice-small".to_string()),
                hot_reloadable: true,
                description: "STT transcription model name",
                validator: validate_non_empty_string,
            },
            SettingDefinition {
                key: SettingKey::InferenceSttLanguage,
                value_type: SettingType::String,
                default_value: SettingValue::String("auto".to_string()),
                hot_reloadable: true,
                description: "Default language hint for speech recognition",
                validator: validate_non_empty_string,
            },
            SettingDefinition {
                key: SettingKey::InferenceTtsEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(true),
                hot_reloadable: true,
                description: "Enable/disable !say text-to-speech audio synthesis",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::InferenceTtsBaseUrl,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: true,
                description: "Base URL for TTS inference service",
                validator: validate_optional_url,
            },
            SettingDefinition {
                key: SettingKey::InferenceTtsModel,
                value_type: SettingType::String,
                default_value: SettingValue::String("kokoro".to_string()),
                hot_reloadable: true,
                description: "Text-to-speech model name",
                validator: validate_non_empty_string,
            },
            SettingDefinition {
                key: SettingKey::InferenceTtsVoice,
                value_type: SettingType::String,
                default_value: SettingValue::String("0".to_string()),
                hot_reloadable: true,
                description: "House default TTS speaker voice",
                validator: validate_non_empty_string,
            },
            SettingDefinition {
                key: SettingKey::InferenceTtsDialect,
                value_type: SettingType::String,
                default_value: SettingValue::String("openai".to_string()),
                hot_reloadable: true,
                description: "Dialect format (openai or elevenlabs)",
                validator: validate_tts_dialect,
            },
            SettingDefinition {
                key: SettingKey::InferenceEmbeddingBaseUrl,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: true,
                description: "Base URL for knowledge embedding inference",
                validator: validate_optional_url,
            },
            SettingDefinition {
                key: SettingKey::InferenceEmbeddingModel,
                value_type: SettingType::String,
                default_value: SettingValue::String("multilingual-e5-small".to_string()),
                hot_reloadable: false,
                description: "Active embedding model for note indexing",
                validator: validate_non_empty_string,
            },
            SettingDefinition {
                key: SettingKey::ChannelsDiscordEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(false),
                hot_reloadable: false,
                description: "Enable/disable Discord adapter",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::ChannelsDiscordGuildId,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: false,
                description: "Target Discord Guild ID for instant slash command registration",
                validator: validate_string_max_500,
            },
            SettingDefinition {
                key: SettingKey::ChannelsTelegramEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(false),
                hot_reloadable: false,
                description: "Enable/disable Telegram adapter",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::ChannelsTelegramMode,
                value_type: SettingType::String,
                default_value: SettingValue::String("polling".to_string()),
                hot_reloadable: false,
                description: "Telegram update mode (webhook or polling)",
                validator: validate_telegram_mode,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailEnabled,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(false),
                hot_reloadable: false,
                description: "Enable/disable Email adapter",
                validator: validate_boolean,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailImapUrl,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: false,
                description: "IMAPS endpoint URL",
                validator: validate_optional_url,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailSmtpUrl,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: false,
                description: "SMTP endpoint URL",
                validator: validate_optional_url,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailFrom,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: false,
                description: "Outbound sender email address",
                validator: validate_optional_email,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailAuthservId,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: false,
                description: "Mail receiving server authserv ID for DMARC validation",
                validator: validate_string_max_500,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailPollSeconds,
                value_type: SettingType::Integer,
                default_value: SettingValue::Integer(60),
                hot_reloadable: false,
                description: "IMAP mailbox polling interval (seconds, min 5)",
                validator: validate_email_poll_seconds,
            },
            SettingDefinition {
                key: SettingKey::ChannelsEmailProcessedFolder,
                value_type: SettingType::String,
                default_value: SettingValue::String("".to_string()),
                hot_reloadable: false,
                description: "Optional IMAP folder for processed messages",
                validator: validate_string_max_500,
            },
            SettingDefinition {
                key: SettingKey::RegistrationAllowLinkSignup,
                value_type: SettingType::Boolean,
                default_value: SettingValue::Boolean(true),
                hot_reloadable: true,
                description: "Allow allowlisted chat identities to create accounts via /link",
                validator: validate_boolean,
            },
        ]
    }
}
