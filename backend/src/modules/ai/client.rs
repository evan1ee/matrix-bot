use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use super::models::{AiUserConfig, ChatCompletionRequest, ChatCompletionResponse};

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

#[derive(Debug, Clone, Default)]
pub struct StreamTurnResult {
    pub content: String,
    pub tool_calls: Vec<super::models::ToolCall>,
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
        let env_base = std::env::var("AI_ENDPOINT")
            .or_else(|_| std::env::var("AI_API_BASE"))
            .or_else(|_| std::env::var("AI_BASE_URL"))
            .or_else(|_| std::env::var("OPENAI_API_BASE"))
            .or_else(|_| std::env::var("OPENAI_BASE_URL"))
            .unwrap_or_else(|_| "https://api.openai.com/v1".to_string());
        let env_key = std::env::var("AI_API_KEY")
            .or_else(|_| std::env::var("XAI_API_KEY"))
            .or_else(|_| std::env::var("OPENAI_API_KEY"))
            .ok()
            .filter(|s| !s.trim().is_empty());
        let env_model = std::env::var("AI_MODEL")
            .or_else(|_| std::env::var("XAI_MODEL"))
            .or_else(|_| std::env::var("OPENAI_MODEL"))
            .unwrap_or_else(|_| {
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
}

#[derive(Clone)]
pub struct AiClient {
    http_client: reqwest::Client,
}

impl AiClient {
    pub fn new() -> Self {
        dotenvy::dotenv().ok();
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .pool_idle_timeout(Some(Duration::from_secs(30)))
            .timeout(Duration::from_secs(120));

        if let Ok(proxy_url) = std::env::var("AI_PROXY")
            .or_else(|_| std::env::var("HTTPS_PROXY"))
            .or_else(|_| std::env::var("https_proxy"))
            .or_else(|_| std::env::var("ALL_PROXY"))
            .or_else(|_| std::env::var("all_proxy"))
            .or_else(|_| std::env::var("HTTP_PROXY"))
            .or_else(|_| std::env::var("http_proxy"))
        {
            let trimmed = proxy_url.trim();
            if !trimmed.is_empty() {
                match reqwest::Proxy::all(trimmed) {
                    Ok(proxy) => {
                        info!("AI Assistant client configured with proxy: {}", trimmed);
                        builder = builder.proxy(proxy);
                    }
                    Err(err) => {
                        warn!("Failed to parse AI proxy URL '{}': {}", trimmed, err);
                    }
                }
            }
        }

        let http_client = builder
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

        let mut last_err = None;
        for attempt in 1..=3 {
            debug!(
                "Sending chat completion (attempt {}/3) to {} with model {}",
                attempt, url, request.model
            );

            let res = match self
                .http_client
                .post(&url)
                .headers(headers.clone())
                .json(request)
                .send()
                .await
            {
                Ok(res) => res,
                Err(err) => {
                    warn!(
                        "AI request attempt {} failed with connection error: {}. Retrying...",
                        attempt, err
                    );
                    last_err = Some(err);
                    if attempt < 3 {
                        tokio::time::sleep(Duration::from_millis(500 * attempt)).await;
                    }
                    continue;
                }
            };

            let status = res.status();
            if !status.is_success() {
                let error_text = res
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown API error".to_string());
                error!("AI provider error ({}): {}", status.as_u16(), error_text);
                return Err(AiClientError::Api {
                    status: status.as_u16(),
                    message: error_text,
                });
            }

            let response: ChatCompletionResponse = res.json().await?;
            return Ok(response);
        }

        Err(AiClientError::Http(last_err.unwrap()))
    }

    pub async fn chat_completion_stream<F, Fut>(
        &self,
        config: &ResolvedAiConfig,
        request: &ChatCompletionRequest,
        mut on_delta: F,
    ) -> Result<StreamTurnResult, AiClientError>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let url = format!("{}/chat/completions", config.api_base_url);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );

        if let Some(ref key) = config.api_key {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", key)) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        let mut stream_req = request.clone();
        stream_req.stream = Some(true);

        let res = self
            .http_client
            .post(&url)
            .headers(headers)
            .json(&stream_req)
            .send()
            .await?;

        let status = res.status();
        if !status.is_success() {
            let error_text = res
                .text()
                .await
                .unwrap_or_else(|_| "Unknown API error".to_string());
            error!("AI streaming error ({}): {}", status.as_u16(), error_text);
            return Err(AiClientError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let mut stream = res.bytes_stream();
        let mut buffer = String::new();
        let mut full_content = String::new();
        let mut tool_calls: Vec<super::models::ToolCall> = Vec::new();

        while let Some(item) = stream.next().await {
            let bytes = item?;
            let chunk_str = String::from_utf8_lossy(&bytes);
            buffer.push_str(&chunk_str);

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer.drain(..=newline_pos);

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(chunk) = parse_sse_chunk(&line) {
                    for choice in chunk.choices {
                        if let Some(ref text) = choice.delta.content {
                            if !text.is_empty() {
                                full_content.push_str(text);
                                on_delta(text.clone()).await;
                            }
                        }
                        if let Some(tc_chunks) = choice.delta.tool_calls {
                            for tc in tc_chunks {
                                let idx = tc.index;
                                while tool_calls.len() <= idx {
                                    tool_calls.push(super::models::ToolCall {
                                        id: String::new(),
                                        r#type: "function".to_string(),
                                        function: super::models::FunctionCall {
                                            name: String::new(),
                                            arguments: String::new(),
                                        },
                                    });
                                }
                                if let Some(id) = tc.id {
                                    tool_calls[idx].id.push_str(&id);
                                }
                                if let Some(func) = tc.function {
                                    if let Some(name) = func.name {
                                        tool_calls[idx].function.name.push_str(&name);
                                    }
                                    if let Some(args) = func.arguments {
                                        tool_calls[idx].function.arguments.push_str(&args);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if !buffer.trim().is_empty() {
            if let Some(chunk) = parse_sse_chunk(buffer.trim()) {
                for choice in chunk.choices {
                    if let Some(ref text) = choice.delta.content {
                        if !text.is_empty() {
                            full_content.push_str(text);
                            on_delta(text.clone()).await;
                        }
                    }
                }
            }
        }

        Ok(StreamTurnResult {
            content: full_content,
            tool_calls,
        })
    }

    pub async fn verify_api_key(
        &self,
        base_url: &str,
        api_key: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), String> {
        let clean_base = base_url.trim().trim_end_matches('/');
        if clean_base.is_empty() {
            return Err("API base URL cannot be empty".to_string());
        }

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(key) = api_key.filter(|k| !k.trim().is_empty()) {
            if let Ok(val) = HeaderValue::from_str(&format!("Bearer {}", key.trim())) {
                headers.insert(AUTHORIZATION, val);
            }
        }

        // 1. First try GET /models
        let models_url = format!("{}/models", clean_base);
        match self
            .http_client
            .get(&models_url)
            .headers(headers.clone())
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(res) => {
                let status = res.status();
                if status.is_success() {
                    return Ok(());
                }
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    let body = res.text().await.unwrap_or_default();
                    let snippet: String = body.chars().take(200).collect();
                    return Err(format!(
                        "Invalid API key (HTTP {}): {}",
                        status.as_u16(),
                        snippet
                    ));
                }
            }
            Err(err) => {
                if err.is_connect() || err.is_timeout() {
                    return Err(format!(
                        "Connection error connecting to {}: {}",
                        clean_base, err
                    ));
                }
            }
        }

        // 2. Fallback to minimal chat completion
        let completions_url = format!("{}/chat/completions", clean_base);
        let test_model = model.unwrap_or("gpt-4o-mini");
        let body = serde_json::json!({
            "model": test_model,
            "messages": [{ "role": "user", "content": "ping" }],
            "max_tokens": 1
        });

        let comp_res = self
            .http_client
            .post(&completions_url)
            .headers(headers)
            .json(&body)
            .timeout(Duration::from_secs(12))
            .send()
            .await
            .map_err(|e| format!("Connection error connecting to {}: {}", clean_base, e))?;

        let comp_status = comp_res.status();
        if comp_status.is_success() {
            return Ok(());
        }
        let err_body = comp_res.text().await.unwrap_or_default();
        let snippet: String = err_body.chars().take(200).collect();
        if comp_status == reqwest::StatusCode::UNAUTHORIZED
            || comp_status == reqwest::StatusCode::FORBIDDEN
        {
            return Err(format!(
                "Invalid API key (HTTP {}): {}",
                comp_status.as_u16(),
                snippet
            ));
        }

        Err(format!(
            "Provider error (HTTP {}): {}",
            comp_status.as_u16(),
            snippet
        ))
    }
}

/// Parses a single Server-Sent Events line into a `ChatCompletionChunk`.
pub fn parse_sse_chunk(line: &str) -> Option<super::models::ChatCompletionChunk> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload == "[DONE]" {
        return None;
    }
    serde_json::from_str::<super::models::ChatCompletionChunk>(payload).ok()
}

/// Extracts delta text content from a single Server-Sent Events line.
pub fn parse_sse_line(line: &str) -> Option<String> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload == "[DONE]" {
        return None;
    }

    let chunk = serde_json::from_str::<super::models::ChatCompletionChunk>(payload).ok()?;
    for choice in chunk.choices {
        if let Some(content) = choice.delta.content {
            if !content.is_empty() {
                return Some(content);
            }
        }
    }
    None
}

impl Default for AiClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(not(test))]
static AI_CLIENT: std::sync::OnceLock<AiClient> = std::sync::OnceLock::new();

#[cfg(not(test))]
pub fn get_ai_client() -> AiClient {
    AI_CLIENT.get_or_init(AiClient::new).clone()
}

#[cfg(test)]
pub fn get_ai_client() -> AiClient {
    AiClient::new()
}

#[cfg(test)]
mod tests {
    use super::super::models::ChatCompletionMessage;
    use super::*;

    #[test]
    fn test_parse_sse_line() {
        let chunk_line = r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"Hello world"},"finish_reason":null}]}"#;
        assert_eq!(parse_sse_line(chunk_line), Some("Hello world".to_string()));

        let empty_delta =
            r#"data: {"id":"chatcmpl-1","choices":[{"index":0,"delta":{},"finish_reason":null}]}"#;
        assert_eq!(parse_sse_line(empty_delta), None);

        let done_line = "data: [DONE]";
        assert_eq!(parse_sse_line(done_line), None);

        let comment = ": keep-alive";
        assert_eq!(parse_sse_line(comment), None);
    }

    #[test]
    fn test_parse_sse_chunk_with_tool_calls() {
        let chunk_line = r#"data: {"id":"chatcmpl-2","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_99","type":"function","function":{"name":"search_web","arguments":"{\"query\":\"tokyo\"}"}}]},"finish_reason":null}]}"#;
        let chunk = parse_sse_chunk(chunk_line).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        let tc = &chunk.choices[0].delta.tool_calls.as_ref().unwrap()[0];
        assert_eq!(tc.id.as_deref(), Some("call_99"));
        assert_eq!(
            tc.function.as_ref().unwrap().name.as_deref(),
            Some("search_web")
        );
    }

    #[test]
    fn test_resolved_config_defaults() {
        let resolved = ResolvedAiConfig::from_user_config(None);
        if std::env::var("AI_ENDPOINT").is_err()
            && std::env::var("AI_API_BASE").is_err()
            && std::env::var("OPENAI_API_BASE").is_err()
        {
            assert_eq!(resolved.api_base_url, "https://api.openai.com/v1");
            assert_eq!(resolved.model_name, "gpt-4o-mini");
        }
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

        let resolved = ResolvedAiConfig::from_user_config(Some(&user_config));
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

        let resolved = ResolvedAiConfig::from_user_config(Some(&user_config));
        assert_eq!(resolved.temperature, 2.0);
        assert_eq!(resolved.max_tokens, 8192);
    }

    #[ignore]
    #[tokio::test]
    async fn test_live_xai_verify_and_completion() {
        let root_env = format!("{}/../.env", env!("CARGO_MANIFEST_DIR"));
        let _ = dotenvy::from_path(&root_env);
        let _ = dotenvy::dotenv();
        let config = ResolvedAiConfig::from_user_config(None);
        println!("\n=== Live Test Config ===");
        println!("Resolved base URL: {}", config.api_base_url);
        println!("Resolved model: {}", config.model_name);
        println!("Resolved has API key: {}", config.api_key.is_some());
        let api_key = match config.api_key.as_ref() {
            Some(k) if !k.trim().is_empty() => k.clone(),
            _ => return, // Skip test if no live key is configured
        };

        let client = AiClient::new();
        let verify = client
            .verify_api_key(
                &config.api_base_url,
                Some(&api_key),
                Some(&config.model_name),
            )
            .await;
        assert!(verify.is_ok(), "Live verify failed: {:?}", verify.err());

        let req = ChatCompletionRequest {
            model: config.model_name.clone(),
            messages: vec![
                ChatCompletionMessage {
                    role: "system".to_string(),
                    content: Some(
                        "You are a concise AI assistant. Reply in one short sentence.".to_string(),
                    ),
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                },
                ChatCompletionMessage {
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
            .chat_completion(&config, &req)
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
}
