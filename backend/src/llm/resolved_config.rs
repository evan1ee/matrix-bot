//! Which LLM a request goes to: the env defaults, overridden per user by their
//! saved AI settings. Business logic, so it lives here and hands the transport
//! only the connection half (`endpoint`).

use crate::application::config::RuntimeConfigSnapshot;
use crate::domain::config::SettingKey;
use crate::domain::llm::AiUserConfig;
use crate::domain::secrets::SecretProvider;
use crate::infrastructure::inference::llm::LlmEndpoint;

#[derive(Clone)]
pub struct ResolvedAiConfig {
    pub api_base_url: String,
    pub api_key: Option<String>,
    pub model_name: String,
    pub system_prompt: Option<String>,
    pub temperature: f64,
    pub max_tokens: i64,
    pub request_timeout_seconds: u64,
}

impl std::fmt::Debug for ResolvedAiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedAiConfig")
            .field("api_base_url", &self.api_base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("model_name", &self.model_name)
            .field("system_prompt", &self.system_prompt)
            .field("temperature", &self.temperature)
            .field("max_tokens", &self.max_tokens)
            .field("request_timeout_seconds", &self.request_timeout_seconds)
            .finish()
    }
}

impl ResolvedAiConfig {
    /// Maps a `llm.default_provider` setting value to its exact `(SecretProvider, slot_name)` pair.
    ///
    /// Never falls back across providers so an OpenAI credential is never sent to xAI/DeepSeek/Ollama.
    pub fn secret_slot_for_provider(provider: &str) -> (SecretProvider, &'static str) {
        match provider.trim().to_ascii_lowercase().as_str() {
            "anthropic" => (SecretProvider::Anthropic, "default"),
            "xai" => (SecretProvider::Generic, "xai"),
            "deepseek" => (SecretProvider::Generic, "deepseek"),
            "ollama" => (SecretProvider::Generic, "ollama"),
            "custom" => (SecretProvider::Generic, "custom"),
            _ => (SecretProvider::OpenAi, "default"),
        }
    }

    fn canonical_base_url_for_provider(provider: &str) -> Option<&'static str> {
        match provider.trim().to_ascii_lowercase().as_str() {
            "openai" => Some("https://api.openai.com/v1"),
            "anthropic" => Some("https://api.anthropic.com/v1"),
            "xai" => Some("https://api.x.ai/v1"),
            "deepseek" => Some("https://api.deepseek.com/v1"),
            "ollama" => Some("http://localhost:11434/v1"),
            "custom" => None,
            _ => Some("https://api.openai.com/v1"),
        }
    }

    pub fn from_user_config(
        ai: &crate::config::AiEnvConfig,
        user_config: Option<&AiUserConfig>,
    ) -> Self {
        Self::resolve(ai, None, None, user_config)
    }

    /// Resolves active AI configuration enforcing precedence:
    /// User Config > Runtime Snapshot (when customized in `app_settings` + `provider_secrets`) > Environment (`AiEnvConfig`) > Defaults.
    ///
    /// Security invariant (B3):
    /// - When a user specifies a custom `api_base_url` that differs from the instance endpoint,
    ///   instance secrets (`provider_secret_key` / `ai.api_key`) are never attached.
    /// - When `llm.default_provider` is customized to a different provider than `ai.base_url`,
    ///   `ai.api_key` is not sent to that other provider's endpoint.
    pub fn resolve(
        ai: &crate::config::AiEnvConfig,
        snapshot: Option<&RuntimeConfigSnapshot>,
        provider_secret_key: Option<&str>,
        user_config: Option<&AiUserConfig>,
    ) -> Self {
        let provider_name = snapshot
            .map(|s| s.llm_default_provider())
            .unwrap_or("openai");
        let provider_customized =
            snapshot.is_some_and(|s| s.is_customized(SettingKey::LlmDefaultProvider));

        let env_base = ai
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string());

        let instance_base_url = if provider_customized {
            Self::canonical_base_url_for_provider(provider_name)
                .map(str::to_string)
                .or_else(|| env_base.clone())
                .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
        } else {
            env_base.clone().unwrap_or_else(|| {
                Self::canonical_base_url_for_provider(provider_name)
                    .unwrap_or("https://api.openai.com/v1")
                    .to_string()
            })
        };

        let env_key_compatible =
            if !provider_customized || provider_name.eq_ignore_ascii_case("custom") {
                true
            } else if let Some(ref eb) = env_base {
                eb.eq_ignore_ascii_case(instance_base_url.trim_end_matches('/'))
            } else {
                provider_name.eq_ignore_ascii_case("openai")
            };

        let instance_api_key = provider_secret_key
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                if env_key_compatible {
                    ai.api_key
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                } else {
                    None
                }
            });

        let user_base_url = user_config
            .and_then(|c| c.api_base_url.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string());

        let user_api_key = user_config
            .and_then(|c| c.api_key.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let (api_base_url, api_key) = match user_base_url {
            Some(ref u_base)
                if !u_base.eq_ignore_ascii_case(instance_base_url.trim_end_matches('/')) =>
            {
                // User pointed to a custom endpoint: never attach instance secrets
                (u_base.clone(), user_api_key)
            }
            Some(u_base) => (u_base, user_api_key.or(instance_api_key)),
            None => (instance_base_url.clone(), user_api_key.or(instance_api_key)),
        };

        let customized_snapshot_model = snapshot
            .filter(|s| s.is_customized(SettingKey::LlmDefaultModel))
            .map(|s| s.llm_default_model().to_string());

        let env_or_default_model = ai
            .model
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if api_base_url.contains("x.ai") {
                    "grok-4.6".to_string()
                } else if api_base_url.contains("anthropic.com") {
                    "claude-3-5-sonnet-latest".to_string()
                } else if api_base_url.contains("deepseek.com") {
                    "deepseek-chat".to_string()
                } else {
                    "gpt-4o-mini".to_string()
                }
            });

        let model_name = user_config
            .and_then(|c| c.model_name.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or(customized_snapshot_model)
            .unwrap_or(env_or_default_model);

        let system_prompt = user_config
            .and_then(|c| c.system_prompt.as_ref())
            .filter(|s| !s.trim().is_empty())
            .cloned();

        let default_temperature = snapshot.map(|s| s.llm_default_temperature()).unwrap_or(0.7);
        let temperature = user_config
            .and_then(|c| c.temperature)
            .unwrap_or(default_temperature)
            .clamp(0.0, 2.0);

        let default_max_tokens = snapshot.map(|s| s.llm_default_max_tokens()).unwrap_or(2048);
        let max_tokens = user_config
            .and_then(|c| c.max_tokens)
            .unwrap_or(default_max_tokens)
            .clamp(64, 8192);

        let request_timeout_seconds = snapshot
            .map(|s| s.llm_request_timeout_seconds().clamp(5, 300) as u64)
            .unwrap_or(60);

        Self {
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            api_key,
            model_name,
            system_prompt,
            temperature,
            max_tokens,
            request_timeout_seconds,
        }
    }

    /// The connection half of this config, for `services::llm`.
    pub fn endpoint(&self) -> LlmEndpoint<'_> {
        LlmEndpoint {
            base_url: &self.api_base_url,
            api_key: self.api_key.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_config_defaults() {
        let resolved =
            ResolvedAiConfig::from_user_config(&crate::config::AiEnvConfig::default(), None);
        assert_eq!(resolved.api_base_url, "https://api.openai.com/v1");
        assert_eq!(resolved.model_name, "gpt-4o-mini");
        assert_eq!(resolved.temperature, 0.7);
        assert_eq!(resolved.max_tokens, 2048);
        assert_eq!(resolved.request_timeout_seconds, 60);
    }

    #[test]
    fn test_resolved_config_user_override() {
        let user_config = AiUserConfig {
            owner_id: "acct_123".to_string(),
            api_base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-test1234".to_string()),
            model_name: Some("gpt-4o-mini".to_string()),
            system_prompt: Some("Custom system prompt".to_string()),
            temperature: Some(0.3),
            max_tokens: Some(1024),
            max_history_turns: Some(10),
            updated_at: "2026-08-27T00:00:00Z".to_string(),
            has_api_key: true,
        };

        let resolved = ResolvedAiConfig::from_user_config(
            &crate::config::AiEnvConfig::default(),
            Some(&user_config),
        );
        assert_eq!(resolved.api_base_url, "https://api.openai.com/v1");
        assert_eq!(resolved.api_key.as_deref(), Some("sk-test1234"));
        assert_eq!(resolved.model_name, "gpt-4o-mini");
        assert_eq!(
            resolved.system_prompt.as_deref(),
            Some("Custom system prompt")
        );
        assert_eq!(resolved.temperature, 0.3);
        assert_eq!(resolved.max_tokens, 1024);
    }

    #[test]
    fn test_temperature_and_token_clamping() {
        let user_config = AiUserConfig {
            owner_id: "acct_123".to_string(),
            api_base_url: None,
            api_key: None,
            model_name: None,
            system_prompt: None,
            temperature: Some(5.0),
            max_tokens: Some(99999),
            max_history_turns: None,
            updated_at: "2026-08-27T00:00:00Z".to_string(),
            has_api_key: false,
        };

        let resolved = ResolvedAiConfig::from_user_config(
            &crate::config::AiEnvConfig::default(),
            Some(&user_config),
        );
        assert_eq!(resolved.temperature, 2.0);
        assert_eq!(resolved.max_tokens, 8192);
    }

    #[test]
    fn test_endpoint_borrows_the_base_url_and_key() {
        let config = ResolvedAiConfig {
            api_base_url: "https://api.x.ai/v1".to_string(),
            api_key: Some("sk-test".to_string()),
            model_name: "grok-4.3".to_string(),
            system_prompt: None,
            temperature: 0.7,
            max_tokens: 2048,
            request_timeout_seconds: 60,
        };
        let endpoint = config.endpoint();
        assert_eq!(endpoint.base_url, "https://api.x.ai/v1");
        assert_eq!(endpoint.api_key, Some("sk-test"));

        let keyless = ResolvedAiConfig {
            api_key: None,
            ..config.clone()
        };
        assert_eq!(keyless.endpoint().api_key, None);
    }

    #[test]
    fn test_resolution_precedence() {
        use crate::infrastructure::postgres::repositories::app_settings::SettingRow;
        use chrono::Utc;

        let ai_env = crate::config::AiEnvConfig {
            base_url: Some("https://api.env.com/v1".to_string()),
            api_key: Some("env-key".to_string()),
            model: Some("env-model".to_string()),
            proxy: None,
            allow_private_base_url: false,
        };

        let rows = vec![
            SettingRow {
                key: "llm.default_model".to_string(),
                value: serde_json::json!("snapshot-model"),
                updated_at: Utc::now(),
                updated_by: None,
            },
            SettingRow {
                key: "llm.default_temperature".to_string(),
                value: serde_json::json!(0.2),
                updated_at: Utc::now(),
                updated_by: None,
            },
        ];
        let snapshot = RuntimeConfigSnapshot::from_persisted(&rows);

        // 1. Snapshot overrides env model, provider secret overrides env key
        let resolved =
            ResolvedAiConfig::resolve(&ai_env, Some(&snapshot), Some("secret-key"), None);
        assert_eq!(resolved.model_name, "snapshot-model");
        assert_eq!(resolved.api_key.as_deref(), Some("secret-key"));
        assert_eq!(resolved.temperature, 0.2);

        // 2. User config overrides snapshot and secret
        let user_config = AiUserConfig {
            owner_id: "u1".to_string(),
            api_base_url: None,
            api_key: Some("user-key".to_string()),
            model_name: Some("user-model".to_string()),
            system_prompt: None,
            temperature: Some(0.9),
            max_tokens: Some(4000),
            max_history_turns: None,
            updated_at: Utc::now().to_rfc3339(),
            has_api_key: true,
        };
        let resolved_user = ResolvedAiConfig::resolve(
            &ai_env,
            Some(&snapshot),
            Some("secret-key"),
            Some(&user_config),
        );
        assert_eq!(resolved_user.model_name, "user-model");
        assert_eq!(resolved_user.api_key.as_deref(), Some("user-key"));
        assert_eq!(resolved_user.temperature, 0.9);
        assert_eq!(resolved_user.max_tokens, 4000);
    }

    #[test]
    fn test_uncustomized_snapshot_preserves_env_model_and_isolates_secrets() {
        use crate::infrastructure::postgres::repositories::app_settings::SettingRow;
        use chrono::Utc;

        let ai_env = crate::config::AiEnvConfig {
            base_url: Some("https://api.openai.com/v1".to_string()),
            api_key: Some("sk-env-openai-key".to_string()),
            model: Some("gpt-4o-from-env".to_string()),
            proxy: None,
            allow_private_base_url: false,
        };

        // 1. Default (uncustomized) snapshot must NOT clobber AI_MODEL from env (B4)
        let default_snap = RuntimeConfigSnapshot::default();
        let resolved = ResolvedAiConfig::resolve(&ai_env, Some(&default_snap), None, None);
        assert_eq!(resolved.model_name, "gpt-4o-from-env");
        assert_eq!(resolved.api_key.as_deref(), Some("sk-env-openai-key"));

        // Debug redacts api_key (M16)
        let dbg = format!("{resolved:?}");
        assert!(dbg.contains("[REDACTED]"));
        assert!(!dbg.contains("sk-env-openai-key"));

        // 2. Admin customizing llm.default_provider to anthropic switches endpoint and does not leak OpenAI env key (B3)
        let anthropic_snap = RuntimeConfigSnapshot::from_persisted(&[SettingRow {
            key: "llm.default_provider".to_string(),
            value: serde_json::json!("anthropic"),
            updated_at: Utc::now(),
            updated_by: None,
        }]);
        let resolved_ant = ResolvedAiConfig::resolve(&ai_env, Some(&anthropic_snap), None, None);
        assert_eq!(resolved_ant.api_base_url, "https://api.anthropic.com/v1");
        assert_eq!(resolved_ant.api_key, None);

        // 3. User custom api_base_url without user api_key must never inherit instance vault/env key (B3)
        let custom_url_user = AiUserConfig {
            owner_id: "u2".to_string(),
            api_base_url: Some("https://custom-user-llm.example.com/v1".to_string()),
            api_key: None,
            model_name: None,
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            max_history_turns: None,
            updated_at: Utc::now().to_rfc3339(),
            has_api_key: false,
        };
        let resolved_custom_user = ResolvedAiConfig::resolve(
            &ai_env,
            Some(&default_snap),
            Some("sk-vault-master-key"),
            Some(&custom_url_user),
        );
        assert_eq!(
            resolved_custom_user.api_base_url,
            "https://custom-user-llm.example.com/v1"
        );
        assert_eq!(
            resolved_custom_user.api_key, None,
            "User custom endpoint must never receive instance secret key"
        );
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_verify_and_completion() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::from_path(".env"); // current-directory only; never dotenvy's own parent-directory-searching dotenv() call
        let ai_env = crate::config::AiEnvConfig::from_env();
        let config = ResolvedAiConfig::from_user_config(&ai_env, None);
        println!("\n=== Live Test Config ===");
        println!("Resolved base URL: {}", config.api_base_url);
        println!("Resolved model: {}", config.model_name);
        println!("Resolved has API key: {}", config.api_key.is_some());
        let api_key = match config.api_key.as_ref() {
            Some(k) if !k.trim().is_empty() => k.clone(),
            _ => {
                panic!("live test run without an AI API key: set AI_API_KEY in the repo-root .env")
            }
        };

        let client = crate::infrastructure::inference::llm::LlmClient::new(ai_env.proxy.as_deref());
        let verify = client
            .verify_api_key(
                &config.api_base_url,
                Some(&api_key),
                Some(&config.model_name),
            )
            .await;
        assert!(verify.is_ok(), "Live verify failed: {:?}", verify.err());

        let req = crate::infrastructure::inference::llm::ChatCompletionRequest {
            model: config.model_name.clone(),
            messages: vec![
                crate::infrastructure::inference::llm::ChatCompletionMessage {
                    role: "system".to_string(),
                    content: Some(
                        "You are a concise AI assistant. Reply in one short sentence.".to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                crate::infrastructure::inference::llm::ChatCompletionMessage {
                    role: "user".to_string(),
                    content: Some("State your model name and say hello.".to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            ],
            temperature: Some(0.7),
            max_tokens: Some(100),
            tools: None,
            tool_choice: None,
            stream: Some(false),
        };

        let response = client
            .chat_completion(config.endpoint(), &req)
            .await
            .expect("Live chat completion should succeed");
        let choice = response
            .choices
            .into_iter()
            .next()
            .expect("Should return choice");
        let text = choice.message.content.expect("Should have message content");
        println!("\n=== Live xAI Response ===");
        println!("{}", text);
        assert!(!text.trim().is_empty());
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_streaming_completion() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::from_path(".env"); // current-directory only; never dotenvy's own parent-directory-searching dotenv() call
        let ai_env = crate::config::AiEnvConfig::from_env();
        let config = ResolvedAiConfig::from_user_config(&ai_env, None);
        if config
            .api_key
            .as_deref()
            .is_none_or(|k| k.trim().is_empty())
        {
            panic!("live test run without an AI API key: set AI_API_KEY in the repo-root .env");
        }

        let request = crate::infrastructure::inference::llm::ChatCompletionRequest {
            model: config.model_name.clone(),
            messages: vec![
                crate::infrastructure::inference::llm::ChatCompletionMessage {
                    role: "user".to_string(),
                    content: Some("Count from one to five in words.".to_string()),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
            ],
            temperature: Some(0.0),
            max_tokens: Some(60),
            tools: None,
            tool_choice: None,
            stream: None,
        };
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(0usize));

        let result = crate::infrastructure::inference::llm::LlmClient::new(ai_env.proxy.as_deref())
            .chat_completion_stream(config.endpoint(), &request, |_| {
                let deltas = deltas.clone();
                async move {
                    *deltas.lock().unwrap() += 1;
                }
            })
            .await
            .expect("live streaming completion should succeed");

        assert!(!result.content.trim().is_empty());
        assert!(
            *deltas.lock().unwrap() > 1,
            "a streamed reply arrives in several deltas"
        );
    }
}
