use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;
use thiserror::Error;
use tracing::{debug, error, info, warn};

use super::types::{ChatCompletionRequest, ChatCompletionResponse};

#[derive(Debug, Error)]
pub enum LlmError {
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
pub struct StreamedCompletion {
    pub content: String,
    pub tool_calls: Vec<super::types::ToolCall>,
}

/// Where a request goes and how it authenticates. Borrowed from the caller's
/// config for the length of one call. Deliberately not `Debug`: it holds a key.
#[derive(Clone, Copy)]
pub struct LlmEndpoint<'a> {
    pub base_url: &'a str,
    pub api_key: Option<&'a str>,
}

#[derive(Clone)]
pub struct LlmClient {
    http_client: reqwest::Client,
}

impl LlmClient {
    pub fn new(proxy: Option<&str>) -> Self {
        let mut builder = reqwest::Client::builder()
            .use_rustls_tls()
            .redirect(reqwest::redirect::Policy::none())
            .tcp_keepalive(Some(Duration::from_secs(30)))
            .pool_idle_timeout(Some(Duration::from_secs(30)))
            .timeout(Duration::from_secs(120));

        if let Some(proxy_url) = proxy {
            let trimmed = proxy_url.trim();
            if !trimmed.is_empty() {
                match reqwest::Proxy::all(trimmed) {
                    Ok(proxy) => {
                        info!(
                            "LLM client configured with proxy: {}",
                            display_proxy(trimmed)
                        );
                        builder = builder.proxy(proxy);
                    }
                    Err(err) => {
                        warn!(
                            "Failed to parse LLM proxy URL {}: {}",
                            display_proxy(trimmed),
                            err
                        );
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
        endpoint: LlmEndpoint<'_>,
        request: &ChatCompletionRequest,
    ) -> Result<ChatCompletionResponse, LlmError> {
        let url = format!("{}/chat/completions", endpoint.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

        if let Some(key) = endpoint.api_key {
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
            if status.is_redirection() {
                return Err(LlmError::Api {
                    status: status.as_u16(),
                    message: format!(
                        "Redirects are not allowed for AI endpoints (HTTP {})",
                        status.as_u16()
                    ),
                });
            }
            if !status.is_success() {
                let error_text = res
                    .text()
                    .await
                    .unwrap_or_else(|_| "Unknown API error".to_string());
                error!("AI provider error ({}): {}", status.as_u16(), error_text);
                return Err(LlmError::Api {
                    status: status.as_u16(),
                    message: error_text,
                });
            }

            let response: ChatCompletionResponse = res.json().await?;
            return Ok(response);
        }

        Err(LlmError::Http(last_err.unwrap()))
    }

    pub async fn chat_completion_stream<F, Fut>(
        &self,
        endpoint: LlmEndpoint<'_>,
        request: &ChatCompletionRequest,
        mut on_delta: F,
    ) -> Result<StreamedCompletion, LlmError>
    where
        F: FnMut(String) -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        let url = format!("{}/chat/completions", endpoint.base_url);
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            reqwest::header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );

        if let Some(key) = endpoint.api_key {
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
        if status.is_redirection() {
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: format!(
                    "Redirects are not allowed for AI endpoints (HTTP {})",
                    status.as_u16()
                ),
            });
        }
        if !status.is_success() {
            let error_text = res
                .text()
                .await
                .unwrap_or_else(|_| "Unknown API error".to_string());
            error!("AI streaming error ({}): {}", status.as_u16(), error_text);
            return Err(LlmError::Api {
                status: status.as_u16(),
                message: error_text,
            });
        }

        let mut stream = res.bytes_stream();
        let mut buffer: Vec<u8> = Vec::new();
        let mut full_content = String::new();
        let mut tool_calls: Vec<super::types::ToolCall> = Vec::new();

        while let Some(item) = stream.next().await {
            let bytes = item?;
            buffer.extend_from_slice(&bytes);

            while let Some(newline_pos) = buffer.iter().position(|&b| b == b'\n') {
                let line_bytes: Vec<u8> = buffer.drain(..=newline_pos).collect();
                let line_str = String::from_utf8_lossy(&line_bytes);
                let line = line_str.trim();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(chunk) = parse_sse_chunk(line) {
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
                                    tool_calls.push(super::types::ToolCall {
                                        id: String::new(),
                                        r#type: "function".to_string(),
                                        function: super::types::FunctionCall {
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

        if !buffer.is_empty() {
            let tail_str = String::from_utf8_lossy(&buffer);
            let tail = tail_str.trim();
            if !tail.is_empty() {
                if let Some(chunk) = parse_sse_chunk(tail) {
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
        }

        Ok(StreamedCompletion {
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
                if status.is_redirection() {
                    return Err(format!(
                        "Redirects are not allowed for AI endpoints (HTTP {})",
                        status.as_u16()
                    ));
                }
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
        if comp_status.is_redirection() {
            return Err(format!(
                "Redirects are not allowed for AI endpoints (HTTP {})",
                comp_status.as_u16()
            ));
        }
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

/// A proxy URL as it may be logged: scheme, host and port only. Credentials
/// are never shown, and an unparseable value is not echoed at all.
fn display_proxy(raw: &str) -> String {
    match reqwest::Url::parse(raw) {
        Ok(url) => format!(
            "{}://{}{}",
            url.scheme(),
            url.host_str().unwrap_or("?"),
            url.port().map(|p| format!(":{}", p)).unwrap_or_default()
        ),
        Err(_) => "<unparseable proxy URL>".to_string(),
    }
}

/// Parses a single Server-Sent Events line into a `ChatCompletionChunk`.
fn parse_sse_chunk(line: &str) -> Option<super::types::ChatCompletionChunk> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload == "[DONE]" {
        return None;
    }
    serde_json::from_str::<super::types::ChatCompletionChunk>(payload).ok()
}

#[cfg(not(test))]
static LLM_CLIENT: std::sync::OnceLock<LlmClient> = std::sync::OnceLock::new();

/// The process-wide LLM client, built once so every caller shares one
/// connection pool. The `proxy` of the FIRST call wins for the life of the
/// process; later calls' `proxy` arguments are ignored. Every production
/// caller passes the same `AiEnvConfig::proxy`, so this holds today; a caller
/// that needs a different proxy must build its own `LlmClient::new`.
#[cfg(not(test))]
pub fn shared_client(proxy: Option<&str>) -> LlmClient {
    // Built once and shared: the point of the global is the connection pool, and
    // the proxy cannot change without a restart.
    LLM_CLIENT.get_or_init(|| LlmClient::new(proxy)).clone()
}

/// The process-wide LLM client, built once so every caller shares one
/// connection pool. The `proxy` of the FIRST call wins for the life of the
/// process; later calls' `proxy` arguments are ignored. Every production
/// caller passes the same `AiEnvConfig::proxy`, so this holds today; a caller
/// that needs a different proxy must build its own `LlmClient::new`. In
/// tests, a fresh client is built on every call.
#[cfg(test)]
pub fn shared_client(proxy: Option<&str>) -> LlmClient {
    LlmClient::new(proxy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::inference::llm::{ChatCompletionMessage, ChatCompletionRequest};
    use crate::infrastructure::inference::test_support::spawn_server;
    use axum::{
        http::HeaderMap,
        routing::{get, post},
        Json, Router,
    };
    use std::sync::{Arc, Mutex};

    fn user_message(text: &str) -> ChatCompletionMessage {
        ChatCompletionMessage {
            role: "user".to_string(),
            content: Some(text.to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    fn request(model: &str) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![user_message("ping")],
            temperature: Some(0.5),
            max_tokens: Some(16),
            tools: None,
            tool_choice: None,
            stream: None,
        }
    }

    /// What the fake provider received: the Authorization header, the Accept
    /// header, and the JSON body.
    type Seen = Arc<Mutex<Option<(Option<String>, Option<String>, serde_json::Value)>>>;

    fn record(seen: &Seen, headers: &HeaderMap, body: serde_json::Value) {
        let auth = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let accept = headers
            .get("accept")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        *seen.lock().unwrap() = Some((auth, accept, body));
    }

    #[test]
    fn test_display_proxy_hides_credentials() {
        assert_eq!(
            display_proxy("http://user:secret@proxy.local:8080"),
            "http://proxy.local:8080"
        );
        assert_eq!(
            display_proxy("socks5://proxy.local"),
            "socks5://proxy.local"
        );
        assert_eq!(display_proxy("not a url"), "<unparseable proxy URL>");
        assert!(!display_proxy("http://user:secret@proxy.local:8080").contains("secret"));
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

    #[tokio::test]
    async fn test_chat_completion_posts_to_chat_completions_with_bearer_and_body() {
        let seen: Seen = Arc::default();
        let app = Router::new().route(
            "/chat/completions",
            post({
                let seen = seen.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        record(&seen, &headers, body);
                        Json(serde_json::json!({
                            "id": "c1", "model": "test-model",
                            "choices": [{
                                "index": 0,
                                "message": { "role": "assistant", "content": "pong" },
                                "finish_reason": "stop"
                            }]
                        }))
                    }
                }
            }),
        );
        let base = spawn_server(app).await;

        let response = LlmClient::new(None)
            .chat_completion(
                LlmEndpoint {
                    base_url: &base,
                    api_key: Some("sk-test"),
                },
                &request("test-model"),
            )
            .await
            .expect("completion should succeed");

        assert_eq!(response.choices[0].message.content.as_deref(), Some("pong"));
        let (auth, _accept, body) = seen
            .lock()
            .unwrap()
            .clone()
            .expect("the provider was called");
        assert_eq!(auth.as_deref(), Some("Bearer sk-test"));
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["content"], "ping");
        assert!(
            body.get("stream").is_none(),
            "an unset stream flag is omitted"
        );
    }

    #[tokio::test]
    async fn test_chat_completion_non_success_is_an_api_error_with_status_and_body() {
        let seen: Seen = Arc::default();
        let app = Router::new().route(
            "/chat/completions",
            post({
                let seen = seen.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        record(&seen, &headers, body);
                        (axum::http::StatusCode::TOO_MANY_REQUESTS, "rate limited")
                    }
                }
            }),
        );
        let base = spawn_server(app).await;

        let err = LlmClient::new(None)
            .chat_completion(
                LlmEndpoint {
                    base_url: &base,
                    api_key: None,
                },
                &request("test-model"),
            )
            .await
            .expect_err("a 429 is an error");

        match err {
            LlmError::Api { status, message } => {
                assert_eq!(status, 429);
                assert!(message.contains("rate limited"));
            }
            other => panic!("expected an API error, got {:?}", other),
        }
        let (auth, _accept, _body) = seen
            .lock()
            .unwrap()
            .clone()
            .expect("the provider was called");
        assert_eq!(
            auth, None,
            "a request with no key sends no Authorization header"
        );
    }

    #[tokio::test]
    async fn test_chat_completion_stream_assembles_text_and_tool_calls() {
        const SSE: &str = concat!(
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"add_\",\"arguments\":\"{\\\"a\\\":\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"todo\",\"arguments\":\"1}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let seen: Seen = Arc::default();
        let app = Router::new().route(
            "/chat/completions",
            post({
                let seen = seen.clone();
                move |headers: HeaderMap, Json(body): Json<serde_json::Value>| {
                    let seen = seen.clone();
                    async move {
                        record(&seen, &headers, body);
                        ([("content-type", "text/event-stream")], SSE)
                    }
                }
            }),
        );
        let base = spawn_server(app).await;
        let deltas: Arc<Mutex<Vec<String>>> = Arc::default();

        let result = LlmClient::new(None)
            .chat_completion_stream(
                LlmEndpoint {
                    base_url: &base,
                    api_key: Some("sk-test"),
                },
                &request("test-model"),
                |d| {
                    let deltas = deltas.clone();
                    async move {
                        deltas.lock().unwrap().push(d);
                    }
                },
            )
            .await
            .expect("stream should succeed");

        assert_eq!(
            *deltas.lock().unwrap(),
            vec!["Hel".to_string(), "lo".to_string()]
        );
        assert_eq!(result.content, "Hello");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_1");
        assert_eq!(result.tool_calls[0].function.name, "add_todo");
        assert_eq!(result.tool_calls[0].function.arguments, "{\"a\":1}");
        let (auth, accept, body) = seen
            .lock()
            .unwrap()
            .clone()
            .expect("the provider was called");
        assert_eq!(body["stream"], true, "streaming forces stream=true");
        assert_eq!(auth.as_deref(), Some("Bearer sk-test"));
        assert_eq!(accept.as_deref(), Some("text/event-stream"));
    }

    #[tokio::test]
    async fn test_verify_api_key_accepts_a_200_from_models() {
        let app = Router::new().route("/models", get(|| async { "[]" }));
        let base = spawn_server(app).await;

        let result = LlmClient::new(None)
            .verify_api_key(&base, Some("sk-test"), Some("test-model"))
            .await;

        assert!(result.is_ok(), "{:?}", result);
    }

    #[tokio::test]
    async fn test_verify_api_key_rejects_a_401_as_invalid_key() {
        let app = Router::new().route(
            "/models",
            get(|| async { (axum::http::StatusCode::UNAUTHORIZED, "bad key") }),
        );
        let base = spawn_server(app).await;

        let err = LlmClient::new(None)
            .verify_api_key(&base, Some("sk-wrong"), Some("test-model"))
            .await
            .expect_err("a 401 is an invalid key");

        assert!(err.starts_with("Invalid API key (HTTP 401)"), "{}", err);
    }

    #[tokio::test]
    async fn test_llm_client_refuses_redirects_and_hides_redirect_body() {
        let target_hit = Arc::new(Mutex::new(false));
        let target_hit_clone = target_hit.clone();
        let target_app = Router::new().route(
            "/secret",
            get(move || {
                let hit = target_hit_clone.clone();
                async move {
                    *hit.lock().unwrap() = true;
                    (
                        axum::http::StatusCode::UNAUTHORIZED,
                        "SENSITIVE_INTERNAL_SECRET",
                    )
                }
            })
            .post(move || async move {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    "SENSITIVE_INTERNAL_SECRET",
                )
            }),
        );
        let target_base = spawn_server(target_app).await;
        let redirect_url = format!("{target_base}/secret");

        let redirect_url_1 = redirect_url.clone();
        let redirect_url_2 = redirect_url.clone();
        let redirector = Router::new()
            .route(
                "/models",
                get(move || {
                    let loc = redirect_url_1.clone();
                    async move {
                        (
                            axum::http::StatusCode::FOUND,
                            [(axum::http::header::LOCATION, loc)],
                            "SENSITIVE_REDIRECT_BODY",
                        )
                    }
                }),
            )
            .route(
                "/chat/completions",
                post(move || {
                    let loc = redirect_url_2.clone();
                    async move {
                        (
                            axum::http::StatusCode::TEMPORARY_REDIRECT,
                            [(axum::http::header::LOCATION, loc)],
                            "SENSITIVE_REDIRECT_BODY",
                        )
                    }
                }),
            );
        let base = spawn_server(redirector).await;
        let client = LlmClient::new(None);

        let verify_err = client
            .verify_api_key(&base, Some("sk-test"), Some("test-model"))
            .await
            .expect_err("redirect must be rejected");
        assert!(
            verify_err.contains("Redirects are not allowed"),
            "{verify_err}"
        );
        assert!(!verify_err.contains("SENSITIVE"));
        assert!(
            !*target_hit.lock().unwrap(),
            "redirect target must never be hit"
        );

        let chat_err = client
            .chat_completion(
                LlmEndpoint {
                    base_url: &base,
                    api_key: Some("sk-test"),
                },
                &request("test-model"),
            )
            .await
            .expect_err("redirect must be rejected");
        assert!(!chat_err.to_string().contains("SENSITIVE"));
    }

    #[tokio::test]
    async fn test_chat_completion_stream_preserves_multibyte_utf8_split_across_chunks() {
        let sse_line = "data: {\"id\":\"c\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"你好世界\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let bytes = sse_line.as_bytes().to_vec();
        // Split right inside the 3-byte UTF-8 sequence for '好'
        let split_idx = sse_line.find('好').unwrap() + 1;
        let part1 = bytes[..split_idx].to_vec();
        let part2 = bytes[split_idx..].to_vec();

        let app = Router::new().route(
            "/chat/completions",
            post(move || {
                let p1 = part1.clone();
                let p2 = part2.clone();
                async move {
                    let stream = futures_util::stream::iter(vec![
                        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(p1)),
                        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(p2)),
                    ]);
                    (
                        [("content-type", "text/event-stream")],
                        axum::body::Body::from_stream(stream),
                    )
                }
            }),
        );
        let base = spawn_server(app).await;

        let result = LlmClient::new(None)
            .chat_completion_stream(
                LlmEndpoint {
                    base_url: &base,
                    api_key: None,
                },
                &request("test-model"),
                |_| async {},
            )
            .await
            .expect("stream should succeed");

        assert_eq!(result.content, "你好世界");
    }
}
