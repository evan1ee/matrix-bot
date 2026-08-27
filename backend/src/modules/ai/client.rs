use std::time::Duration;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use thiserror::Error;
use tracing::{debug, error};

use super::models::{ChatCompletionRequest, ChatCompletionResponse, AiUserConfig};

#[derive(Debug, Error)]
pub enum AiClientError {
    #[error("HTTP request error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error ({status}): {message}")]
    Api { status: u16, message: String },
    #[error("JSON deserialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Invalid configuration: {0}")]
    Config(String),
}

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
    pub fn from_user_config(user_config: Option<&AiUserConfig>) -> Self {
        let env_base = std::env::var("AI_API_BASE")
            .unwrap_or_else(|_| "http://localhost:11434/v1".to_string());
        let env_key = std::env::var("AI_API_KEY").ok().filter(|s| !s.trim().is_empty());
        let env_model = std::env::var("AI_MODEL")
            .unwrap_or_else(|_| "llama3.2".to_string());

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
}

#[derive(Clone)]
pub struct AiClient {
    http_client: reqwest::Client,
}

impl AiClient {
    pub fn new() -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to build HTTP client for AI assistant");

        Self { http_client }
    }

    pub async fn chat_completion(
        &self,
        config: &ResolvedAiConfig,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, AiClientError> {
        let url = format!("{}/chat/completions", config.api_base_url);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(ref key) = config.api_key {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", key)) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        debug!("Sending chat completion to {} with model {}", url, request.model);

        let res = self
            .http_client
            .post(&url)
            .headers(headers)
            .json(request)
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let error_text = res.text().await.unwrap_or_else(|_| "Unknown API error".to_string());
            error!("AI provider error ({}): {}", status.as_u16(), error_text);
            return Err(AiClientError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let response: ChatCompletionResponse = res.json().await?;
        Ok(response)
    }
}

impl Default for AiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolved_config_defaults() {
        let resolved = ResolvedAiConfig::from_user_config(None);
        assert!(!resolved.api_base_url.is_empty());
        assert!(!resolved.model_name.is_empty());
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
            updated_at: "2026-08-27T00:00:00Z".to_string(),
            has_api_key: true,
        };

        let resolved = ResolvedAiConfig::from_user_config(Some(&user_config));
        assert_eq!(resolved.api_base_url, "https://api.openai.com/v1");
        assert_eq!(resolved.api_key.as_deref(), Some("sk-test1234"));
        assert_eq!(resolved.model_name, "gpt-4o-mini");
        assert_eq!(resolved.system_prompt.as_deref(), Some("Custom system prompt"));
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
            updated_at: "2026-08-27T00:00:00Z".to_string(),
            has_api_key: false,
        };

        let resolved = ResolvedAiConfig::from_user_config(Some(&user_config));
        assert_eq!(resolved.temperature, 2.0);
        assert_eq!(resolved.max_tokens, 8192);
    }
}
