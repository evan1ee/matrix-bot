use super::resolved_config::ResolvedAiConfig;
use crate::domain::chat::QuotedMessage;

/// Returns true if the active AI configuration is using xAI (Grok).
pub fn is_grok_model(config: &ResolvedAiConfig) -> bool {
    let base_lower = config.api_base_url.to_lowercase();
    let model_lower = config.model_name.to_lowercase();
    base_lower.contains("x.ai") || model_lower.contains("grok")
}

/// Returns true if the current request warrants enabling live web search.
///
/// For normal tasks (e.g. adding to-dos, creating reminders, finance tracking, general coding,
/// everyday reasoning, or routine explanations), this returns `false` to prevent search latency,
/// extra token usage, and unnecessary tool invocations.
///
/// For fact checking, verifying claims, breaking news, or explicit search requests,
/// this returns `true`.
pub fn should_enable_web_search(
    _config: &ResolvedAiConfig,
    prompt: &str,
    quoted: Option<&QuotedMessage>,
) -> bool {
    let prompt_lower = prompt.to_lowercase();
    let prompt_trimmed = prompt_lower.trim();

    // 1. Explicit bypass for obvious workspace action commands
    if is_workspace_action_command(prompt_trimmed) {
        return false;
    }

    // 2. Check for fact-checking / verification intent
    if has_fact_check_intent(prompt_trimmed) {
        return true;
    }

    // 3. Check for live web / current information search intent
    if has_live_search_intent(prompt_trimmed) {
        return true;
    }

    // 4. If replying to a quoted message with a verification query
    if let Some(q) = quoted {
        if has_quoted_verification_query(prompt_trimmed, &q.body) {
            return true;
        }
    }

    false
}

/// Backward compatibility alias
pub fn should_enable_grok_search(
    config: &ResolvedAiConfig,
    prompt: &str,
    quoted: Option<&QuotedMessage>,
) -> bool {
    should_enable_web_search(config, prompt, quoted)
}

/// Returns the standard function tool definition for `web_search`.
///
/// Using `type: "function"` ensures 100% compatibility across all OpenAI-compatible
/// endpoints (including xAI Grok, OpenAI gpt-4o, and local models) without 422 schema errors.
pub fn web_search_tool_definition() -> serde_json::Value {
    serde_json::json!({
        "type": "function",
        "function": {
            "name": "web_search",
            "description": "Search the live web for real-time information, recent news, fact-checking, or current events. Use this when the user asks to fact-check, verify claims, search for news, or look up information.",
            "parameters": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query keywords"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }
        }
    })
}

/// Backward compatibility: returns the function-based search tool
pub fn grok_search_tools() -> Vec<serde_json::Value> {
    vec![web_search_tool_definition()]
}

/// Returns the configured Tavily API key from environment, checking `TAVILY_API_KEY`.
pub fn get_tavily_api_key() -> Option<String> {
    std::env::var("TAVILY_API_KEY")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|k| !k.is_empty())
}

/// Executes a live web search using active application services.
///
/// Inspects `search.tavily.enabled` and decrypted secret key from `services.secrets`
/// before falling back to `TAVILY_API_KEY` environment variable or DuckDuckGo.
pub async fn execute_web_search_with_services(
    services: &crate::application::AppServices,
    query: &str,
) -> Result<String, String> {
    let query_trimmed = query.trim();
    if query_trimmed.is_empty() {
        return Err("Search query cannot be empty".to_string());
    }

    let snapshot = services.app_config.snapshot();
    let tavily_enabled = snapshot.search_tavily_enabled();
    let timeout_secs = snapshot.search_timeout_seconds().clamp(1, 120) as u64;

    if tavily_enabled {
        let tavily_key = match services
            .secrets
            .get_decrypted_secret(crate::domain::secrets::SecretProvider::Tavily, "default")
            .await
        {
            Ok(Some(sec)) => Some(sec.expose_secret().to_string()),
            _ => get_tavily_api_key(),
        };

        if let Some(api_key) = tavily_key {
            tracing::info!(
                "Executing web search via Tavily API for query: '{}'",
                query_trimmed
            );
            return execute_tavily_search_with_timeout(query_trimmed, &api_key, timeout_secs).await;
        }
    }

    tracing::info!(
        "Tavily disabled or API key absent; using DuckDuckGo fallback for: '{}'",
        query_trimmed
    );
    execute_duckduckgo_search(query_trimmed).await
}

/// Executes a live web search, using Tavily if `TAVILY_API_KEY` is set,
/// otherwise falling back to DuckDuckGo HTML scraping.
pub async fn execute_web_search(query: &str) -> Result<String, String> {
    let query_trimmed = query.trim();
    if query_trimmed.is_empty() {
        return Err("Search query cannot be empty".to_string());
    }

    if let Some(api_key) = get_tavily_api_key() {
        tracing::info!(
            "Executing web search via Tavily API for query: '{}'",
            query_trimmed
        );
        return execute_tavily_search(query_trimmed, &api_key).await;
    }

    tracing::info!(
        "No Tavily API key set (checked TAVILY_API_KEY); using DuckDuckGo fallback for: '{}'",
        query_trimmed
    );
    execute_duckduckgo_search(query_trimmed).await
}

fn build_search_http_client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    let mut builder =
        reqwest::Client::builder().timeout(std::time::Duration::from_secs(timeout_secs));

    let proxy_url = std::env::var("AI_PROXY")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("https_proxy"))
        .or_else(|_| std::env::var("ALL_PROXY"))
        .or_else(|_| std::env::var("all_proxy"))
        .ok()
        .filter(|s| !s.trim().is_empty());

    if let Some(proxy_str) = proxy_url {
        let trimmed = proxy_str.trim();
        if let Ok(proxy) = reqwest::Proxy::all(trimmed) {
            tracing::debug!("Web search client configured with proxy: {}", trimmed);
            builder = builder.proxy(proxy);
        }
    }

    builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

/// Executes a web search via the Tavily API (optimized for AI/LLM consumption).
async fn execute_tavily_search(query: &str, api_key: &str) -> Result<String, String> {
    execute_tavily_search_with_timeout(query, api_key, 15).await
}

async fn execute_tavily_search_with_timeout(
    query: &str,
    api_key: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let client = build_search_http_client(timeout_secs)?;

    let body = serde_json::json!({
        "api_key": api_key,
        "query": query,
        "search_depth": "basic",
        "max_results": 5,
        "include_answer": false
    });

    let resp = client
        .post("https://api.tavily.com/search")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Tavily search request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let error_body = resp.text().await.unwrap_or_default();
        tracing::warn!(
            "Tavily API error ({}): {}, falling back to DuckDuckGo",
            status,
            error_body
        );
        return execute_duckduckgo_search(query).await;
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse Tavily response: {}", e))?;

    let results = json["results"].as_array();
    let empty_vec = Vec::new();
    let items = results.unwrap_or(&empty_vec);

    if items.is_empty() {
        tracing::info!("Tavily search returned 0 results for '{}'", query);
        return Ok(format!(
            "No web search results found for query: '{}'. Analyze based on prior knowledge.",
            query
        ));
    }

    tracing::info!(
        "Tavily search returned {} results for '{}'",
        items.len(),
        query
    );

    let mut output = format!("Top web search results for '{}' (via Tavily):\n\n", query);
    for (i, result) in items.iter().take(5).enumerate() {
        let title = result["title"].as_str().unwrap_or("Untitled");
        let url = result["url"].as_str().unwrap_or("");
        let content = result["content"].as_str().unwrap_or("");
        let score = result["score"].as_f64().unwrap_or(0.0);
        output.push_str(&format!(
            "{}. **{}** (relevance: {:.0}%)\n   {}\n   Source: {}\n\n",
            i + 1,
            title,
            score * 100.0,
            content,
            url,
        ));
    }

    Ok(output)
}

/// Executes a web search via DuckDuckGo HTML scraping (fallback).
async fn execute_duckduckgo_search(query: &str) -> Result<String, String> {
    tracing::info!(
        "Executing web search via DuckDuckGo fallback for query: '{}'",
        query
    );
    let client = build_search_http_client(10)?;

    let resp = client
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .send()
        .await
        .map_err(|e| format!("Web search request failed: {}", e))?;

    let html = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read search response body: {}", e))?;

    let results = parse_duckduckgo_html(&html);

    if results.is_empty() {
        return Ok(format!(
            "No web search results found for query: '{}'. Analyze based on prior knowledge.",
            query
        ));
    }

    let mut output = format!(
        "Top web search results for '{}' (via DuckDuckGo):\n\n",
        query
    );
    for (i, (title, snippet)) in results.iter().take(5).enumerate() {
        output.push_str(&format!("{}. **{}**\n   {}\n\n", i + 1, title, snippet));
    }

    Ok(output)
}

/// Parses DuckDuckGo HTML results into (title, snippet) pairs.
pub fn parse_duckduckgo_html(html: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut cur_title = None;

    for line in html.lines() {
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find("class=\"result__a\"") {
            let rest = &trimmed[pos..];
            if let Some(open_tag_end) = rest.find('>') {
                let tag_content = &rest[open_tag_end + 1..];
                if let Some(close_tag) = tag_content.find("</a>") {
                    let raw_title = &tag_content[..close_tag];
                    let clean_title = crate::utils::strip_html(raw_title).trim().to_string();
                    if !clean_title.is_empty() {
                        cur_title = Some(clean_title);
                    }
                }
            }
        } else if let Some(pos) = trimmed.find("class=\"result__snippet\"") {
            let rest = &trimmed[pos..];
            if let Some(open_tag_end) = rest.find('>') {
                let tag_content = &rest[open_tag_end + 1..];
                if let Some(close_tag) = tag_content.find("</a>") {
                    let raw_snippet = &tag_content[..close_tag];
                    let clean_snippet = crate::utils::strip_html(raw_snippet).trim().to_string();
                    if !clean_snippet.is_empty() {
                        let title = cur_title
                            .take()
                            .unwrap_or_else(|| "Search Result".to_string());
                        results.push((title, clean_snippet));
                    }
                }
            }
        }
    }

    results
}

fn is_workspace_action_command(p: &str) -> bool {
    p.starts_with("add todo")
        || p.starts_with("todo add")
        || p.starts_with("create todo")
        || p.starts_with("add task")
        || p.starts_with("remind me")
        || p.starts_with("set reminder")
        || p.starts_with("record expense")
        || p.starts_with("add expense")
        || p.starts_with("record income")
        || p.starts_with("add income")
        || p.starts_with("log habit")
        || p.starts_with("save bookmark")
}

fn has_fact_check_intent(p: &str) -> bool {
    p.contains("fact check")
        || p.contains("factcheck")
        || p.contains("fact-check")
        || p.contains("verify")
        || p.contains("true or false")
        || p.contains("is it true")
        || p.contains("is this true")
        || p.contains("is that true")
        || p.contains("debunk")
        || p.contains("rumor")
        || p.contains("fake news")
        || p.contains("accurate")
        || p.contains("check claim")
        || p.contains("check this")
        || p.contains("is this real")
        || p.contains("is it real")
}

fn has_live_search_intent(p: &str) -> bool {
    p.contains("search web")
        || p.contains("web search")
        || p.contains("google this")
        || p.contains("google ")
        || p.starts_with("search ")
        || p.starts_with("search:")
        || p.contains("latest news")
        || p.contains("breaking news")
        || p.contains("recent news")
        || p.contains("trending on x")
        || p.contains("trending on twitter")
        || p.contains("what happened today")
        || p.contains("what happened recently")
        || p.contains("who won today")
        || p.contains("current stock price")
        || p.contains("current price of")
        || p.contains("look up ")
}

fn has_quoted_verification_query(prompt_trimmed: &str, _quoted_body: &str) -> bool {
    prompt_trimmed == "true?"
        || prompt_trimmed == "is this true?"
        || prompt_trimmed == "is this real?"
        || prompt_trimmed == "real?"
        || prompt_trimmed == "fake?"
        || prompt_trimmed == "legit?"
        || prompt_trimmed == "check"
        || prompt_trimmed == "check this"
        || prompt_trimmed == "fact check"
        || prompt_trimmed == "verify"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xai_config() -> ResolvedAiConfig {
        ResolvedAiConfig {
            api_base_url: "https://api.x.ai/v1".to_string(),
            api_key: Some("test-key".to_string()),
            model_name: "grok-4.6".to_string(),
            system_prompt: None,
            temperature: 0.7,
            max_tokens: 2048,
        }
    }

    #[test]
    fn test_is_grok_model() {
        assert!(is_grok_model(&xai_config()));
    }

    #[test]
    fn test_web_search_enabled_on_fact_checking() {
        let cfg = xai_config();
        assert!(should_enable_web_search(
            &cfg,
            "fact check this statement",
            None
        ));
        assert!(should_enable_web_search(
            &cfg,
            "Please verify if this is accurate",
            None
        ));
        assert!(should_enable_web_search(
            &cfg,
            "is it true that NASA found life?",
            None
        ));
        assert!(should_enable_web_search(
            &cfg,
            "search: latest SpaceX launch",
            None
        ));
        assert!(should_enable_web_search(
            &cfg,
            "what is the latest news on AI?",
            None
        ));
    }

    #[test]
    fn test_web_search_enabled_on_quoted_message() {
        let cfg = xai_config();
        let quoted = QuotedMessage {
            author: Some("alice".to_string()),
            body: "The prime minister just resigned unexpectedly.".to_string(),
            event_id: None,
        };

        assert!(should_enable_web_search(&cfg, "true?", Some(&quoted)));
        assert!(should_enable_web_search(
            &cfg,
            "is this real?",
            Some(&quoted)
        ));
        assert!(should_enable_web_search(&cfg, "fact check", Some(&quoted)));
    }

    #[test]
    fn test_web_search_disabled_for_normal_tasks() {
        let cfg = xai_config();
        assert!(!should_enable_web_search(
            &cfg,
            "add todo buy milk tomorrow",
            None
        ));
        assert!(!should_enable_web_search(
            &cfg,
            "remind me in 10 minutes to check oven",
            None
        ));
        assert!(!should_enable_web_search(
            &cfg,
            "record expense 25 USD lunch",
            None
        ));
        assert!(!should_enable_web_search(
            &cfg,
            "How do I implement binary search in Rust?",
            None
        ));
        assert!(!should_enable_web_search(
            &cfg,
            "Translate this text to French: Hello",
            None
        ));
        assert!(!should_enable_web_search(
            &cfg,
            "Draft a friendly email to my team",
            None
        ));
    }

    #[test]
    fn test_get_tavily_api_key() {
        std::env::remove_var("TAVILY_API_KEY");
        assert_eq!(get_tavily_api_key(), None);

        std::env::set_var("TAVILY_API_KEY", "key_plain");
        assert_eq!(get_tavily_api_key(), Some("key_plain".to_string()));

        std::env::remove_var("TAVILY_API_KEY");
    }

    #[test]
    fn test_parse_duckduckgo_html() {
        let sample_html = r#"
            <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com">Example Domain</a>
            <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com">This domain is for use in illustrative examples in documents.</a>
        "#;
        let parsed = parse_duckduckgo_html(sample_html);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, "Example Domain");
        assert_eq!(
            parsed[0].1,
            "This domain is for use in illustrative examples in documents."
        );
    }
}
