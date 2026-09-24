//! Which LLM a request goes to: the env defaults, overridden per user by their
//! saved AI settings. Business logic, so it lives here and hands the transport
//! only the connection half (`endpoint`).

use crate::services::llm::LlmEndpoint;

use super::models::AiUserConfig;

#[derive(Debug, Clone)]
pub struct ResolvedAiConfig {
    pub api_base_url: String,
    pub api_key: Option<String>,
    pub model_name: String,
    pub system_prompt: Option<String>,
    pub temperature: f64,
    pub max_tokens: i64,
}

impl ResolvedAiConfig {
    pub fn from_user_config(
        ai: &crate::config::AiEnvConfig,
        user_config: Option<&AiUserConfig>,
    ) -> Self {
        let env_base = ai
            .base_url
            .clone()
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let env_key = ai.api_key.clone();
        let env_model = ai.model.clone().unwrap_or_else(|| {
            if env_base.contains("x.ai") {
                "grok-4.6".to_string()
            } else {
                "gpt-4o-mini".to_string()
            }
        });

        let api_base_url = user_config
            .and_then(|c| c.api_base_url.as_ref())
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or(env_base);

        let api_key = user_config
            .and_then(|c| c.api_key.as_ref())
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .or(env_key);

        let model_name = user_config
            .and_then(|c| c.model_name.as_ref())
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or(env_model);

        let system_prompt = user_config
            .and_then(|c| c.system_prompt.as_ref())
            .filter(|s| !s.trim().is_empty())
            .cloned();

        let temperature = user_config
            .and_then(|c| c.temperature)
            .unwrap_or(0.7)
            .clamp(0.0, 2.0);

        let max_tokens = user_config
            .and_then(|c| c.max_tokens)
            .unwrap_or(2048)
            .clamp(64, 8192);

        Self {
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
            api_key,
            model_name,
            system_prompt,
            temperature,
            max_tokens,
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

        let client = crate::services::llm::LlmClient::new(ai_env.proxy.as_deref());
        let verify = client
            .verify_api_key(
                &config.api_base_url,
                Some(&api_key),
                Some(&config.model_name),
            )
            .await;
        assert!(verify.is_ok(), "Live verify failed: {:?}", verify.err());

        let req = crate::services::llm::ChatCompletionRequest {
            model: config.model_name.clone(),
            messages: vec![
                crate::services::llm::ChatCompletionMessage {
                    role: "system".to_string(),
                    content: Some(
                        "You are a concise AI assistant. Reply in one short sentence.".to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                crate::services::llm::ChatCompletionMessage {
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
            .map_or(true, |k| k.trim().is_empty())
        {
            panic!("live test run without an AI API key: set AI_API_KEY in the repo-root .env");
        }

        let request = crate::services::llm::ChatCompletionRequest {
            model: config.model_name.clone(),
            messages: vec![crate::services::llm::ChatCompletionMessage {
                role: "user".to_string(),
                content: Some("Count from one to five in words.".to_string()),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            }],
            temperature: Some(0.0),
            max_tokens: Some(60),
            tools: None,
            tool_choice: None,
            stream: None,
        };
        let deltas = std::sync::Arc::new(std::sync::Mutex::new(0usize));

        let result = crate::services::llm::LlmClient::new(ai_env.proxy.as_deref())
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
