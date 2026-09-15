use super::client::ResolvedAiConfig;
use crate::bot::quote::QuotedMessage;

/// Returns true if the active AI configuration is using xAI (Grok).
pub fn is_grok_model(config: &ResolvedAiConfig) -> bool {
    let base_lower = config.api_base_url.to_lowercase();
    let model_lower = config.model_name.to_lowercase();
    base_lower.contains("x.ai") || model_lower.contains("grok")
}

/// Returns true if the current request warrants enabling live web/X search for Grok.
///
/// For normal tasks (e.g. adding to-dos, creating reminders, finance tracking, general coding,
/// everyday reasoning, or routine explanations), this returns `false` to prevent search latency,
/// extra token usage, and unnecessary tool invocations.
///
/// For fact checking, verifying claims, breaking news, or explicit search requests,
/// this returns `true`.
pub fn should_enable_grok_search(
    config: &ResolvedAiConfig,
    prompt: &str,
    quoted: Option<&QuotedMessage>,
) -> bool {
    // 1. Must be a Grok / xAI model
    if !is_grok_model(config) {
        return false;
    }

    let prompt_lower = prompt.to_lowercase();
    let prompt_trimmed = prompt_lower.trim();

    // 2. Explicit bypass for obvious workspace action commands
    if is_workspace_action_command(prompt_trimmed) {
        return false;
    }

    // 3. Check for fact-checking / verification intent
    if has_fact_check_intent(prompt_trimmed) {
        return true;
    }

    // 4. Check for live web / current information search intent
    if has_live_search_intent(prompt_trimmed) {
        return true;
    }

    // 5. If replying to a quoted message with a verification query
    if let Some(q) = quoted {
        if has_quoted_verification_query(prompt_trimmed, &q.body) {
            return true;
        }
    }

    false
}

/// Returns the xAI server-side search tools (`web_search` and `x_search`).
pub fn grok_search_tools() -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({ "type": "web_search" }),
        serde_json::json!({ "type": "x_search" }),
    ]
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
    // If user quotes a message and simply says: "true?", "real?", "check", "check please"
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

    fn openai_config() -> ResolvedAiConfig {
        ResolvedAiConfig {
            api_base_url: "https://api.openai.com/v1".to_string(),
            api_key: Some("test-key".to_string()),
            model_name: "gpt-4o-mini".to_string(),
            system_prompt: None,
            temperature: 0.7,
            max_tokens: 2048,
        }
    }

    #[test]
    fn test_is_grok_model() {
        assert!(is_grok_model(&xai_config()));

        let custom_grok = ResolvedAiConfig {
            api_base_url: "https://custom-gateway.internal/v1".to_string(),
            api_key: None,
            model_name: "grok-2-1212".to_string(),
            system_prompt: None,
            temperature: 0.7,
            max_tokens: 2048,
        };
        assert!(is_grok_model(&custom_grok));

        assert!(!is_grok_model(&openai_config()));
    }

    #[test]
    fn test_grok_search_enabled_on_fact_checking() {
        let cfg = xai_config();
        assert!(should_enable_grok_search(&cfg, "fact check this statement", None));
        assert!(should_enable_grok_search(&cfg, "Please verify if this is accurate", None));
        assert!(should_enable_grok_search(&cfg, "is it true that NASA found life?", None));
        assert!(should_enable_grok_search(&cfg, "search: latest SpaceX launch", None));
        assert!(should_enable_grok_search(&cfg, "what is the latest news on AI?", None));
    }

    #[test]
    fn test_grok_search_enabled_on_quoted_message() {
        let cfg = xai_config();
        let quoted = QuotedMessage {
            author: Some("alice".to_string()),
            body: "The prime minister just resigned unexpectedly.".to_string(),
            event_id: None,
        };

        assert!(should_enable_grok_search(&cfg, "true?", Some(&quoted)));
        assert!(should_enable_grok_search(&cfg, "is this real?", Some(&quoted)));
        assert!(should_enable_grok_search(&cfg, "fact check", Some(&quoted)));
    }

    #[test]
    fn test_grok_search_disabled_for_normal_tasks() {
        let cfg = xai_config();
        assert!(!should_enable_grok_search(&cfg, "add todo buy milk tomorrow", None));
        assert!(!should_enable_grok_search(&cfg, "remind me in 10 minutes to check oven", None));
        assert!(!should_enable_grok_search(&cfg, "record expense 25 USD lunch", None));
        assert!(!should_enable_grok_search(&cfg, "How do I implement binary search in Rust?", None));
        assert!(!should_enable_grok_search(&cfg, "Translate this text to French: Hello", None));
        assert!(!should_enable_grok_search(&cfg, "Draft a friendly email to my team", None));
    }

    #[test]
    fn test_grok_search_disabled_for_non_grok_providers() {
        let cfg = openai_config();
        // Even if fact-check prompt is given, OpenAI endpoints must not receive server-side xAI tools
        assert!(!should_enable_grok_search(&cfg, "fact check this statement", None));
        assert!(!should_enable_grok_search(&cfg, "latest news", None));
    }
}
