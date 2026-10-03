//! AI provider settings read from the environment.

use std::env;

/// AI provider settings that come from the environment.
///
/// The endpoint, key and model each have exactly one name. Resolved once at
/// boot so every caller sees the same answer.
#[derive(Debug, Clone, Default)]
pub struct AiEnvConfig {
    /// `AI_ENDPOINT`: the OpenAI-compatible base URL, including `/v1`.
    pub base_url: Option<String>,
    /// `AI_API_KEY`, ignoring a blank value.
    pub api_key: Option<String>,
    /// `AI_MODEL`.
    pub model: Option<String>,
    /// First of AI_PROXY, HTTPS_PROXY, https_proxy, ALL_PROXY, all_proxy,
    /// HTTP_PROXY, http_proxy.
    pub proxy: Option<String>,
    /// Whether an AI base URL may point at a private or loopback address.
    ///
    /// Off by default: a user-supplied base URL is a server-side request this
    /// process will make, so an unguarded one reaches anything the host can.
    pub allow_private_base_url: bool,
}

impl AiEnvConfig {
    pub fn from_env() -> Self {
        let base_url = env::var("AI_ENDPOINT").ok();

        let api_key = env::var("AI_API_KEY").ok().filter(|s| !s.trim().is_empty());

        let model = env::var("AI_MODEL").ok();

        let proxy = env::var("AI_PROXY")
            .or_else(|_| env::var("HTTPS_PROXY"))
            .or_else(|_| env::var("https_proxy"))
            .or_else(|_| env::var("ALL_PROXY"))
            .or_else(|_| env::var("all_proxy"))
            .or_else(|_| env::var("HTTP_PROXY"))
            .or_else(|_| env::var("http_proxy"))
            .ok();

        let allow_private_base_url = env::var("AI_ALLOW_PRIVATE_BASE_URL")
            .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
            .unwrap_or(false);

        Self {
            base_url,
            api_key,
            model,
            proxy,
            allow_private_base_url,
        }
    }
}
