use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct AiUserConfig {
    pub owner_id: String,
    pub api_base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    pub model_name: Option<String>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    #[serde(default)]
    #[sqlx(default)]
    pub max_history_turns: Option<i64>,
    pub updated_at: String,
    #[sqlx(default)]
    pub has_api_key: bool,
}

impl AiUserConfig {
    pub fn redacted(mut self) -> Self {
        self.has_api_key = self
            .api_key
            .as_ref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false);
        self.api_key = None;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VerifyAiKeyInput {
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
}

pub fn validate_ai_base_url(url_str: &str) -> Result<(), &'static str> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.len() > 500 {
        return Err("API base URL exceeds 500 characters");
    }
    let parsed = url::Url::parse(trimmed).map_err(|_| "API base URL is not a valid URL")?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("API base URL must use http or https scheme");
    }

    let host_str = parsed
        .host_str()
        .ok_or("API base URL must contain a valid host")?;
    let host_lower = host_str.to_ascii_lowercase();

    if host_lower == "169.254.169.254"
        || host_lower.starts_with("169.254.")
        || host_lower.starts_with("fe80:")
        || host_lower.starts_with("[fe80:")
    {
        return Err("API base URL cannot target link-local metadata addresses");
    }

    let allow_private = std::env::var("AI_ALLOW_PRIVATE_BASE_URL")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if !allow_private {
        if host_lower.ends_with(".internal")
            || host_lower.ends_with(".local")
            || host_lower == "localhost"
            || host_lower.ends_with(".localhost")
        {
            return Err("API base URL cannot target internal or local domain names");
        }

        let ip = match parsed.host() {
            Some(url::Host::Ipv4(ip)) => Some(std::net::IpAddr::V4(ip)),
            Some(url::Host::Ipv6(ip)) => Some(std::net::IpAddr::V6(ip)),
            _ => None,
        };

        if let Some(ip) = ip {
            let is_disallowed = match ip {
                std::net::IpAddr::V4(v4) => {
                    v4.is_loopback()
                        || v4.is_private()
                        || v4.is_link_local()
                        || v4.is_broadcast()
                        || v4.is_documentation()
                        || v4.octets()[0] == 0
                        || (v4.octets()[0] == 100 && (v4.octets()[1] & 0xC0) == 64)
                }
                std::net::IpAddr::V6(v6) => {
                    v6.is_loopback()
                        || v6.is_unspecified()
                        || (v6.segments()[0] & 0xffc0) == 0xfe80
                        || (v6.segments()[0] & 0xfe00) == 0xfc00
                }
            };

            if is_disallowed {
                return Err("API base URL cannot target private or loopback IP addresses");
            }
        }
    }

    Ok(())
}

impl VerifyAiKeyInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(ref url_str) = self.api_base_url {
            validate_ai_base_url(url_str)?;
        }
        if let Some(ref key) = self.api_key {
            if key.len() > 500 {
                return Err("API key exceeds 500 characters");
            }
        }
        if let Some(ref model) = self.model_name {
            if model.len() > 500 {
                return Err("Model name exceeds 500 characters");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VerifyAiKeyResponse {
    pub valid: bool,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateAiConfigInput {
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
    pub system_prompt: Option<String>,
    pub temperature: Option<f64>,
    pub max_tokens: Option<i64>,
    pub max_history_turns: Option<i64>,
}

impl UpdateAiConfigInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(ref url_str) = self.api_base_url {
            validate_ai_base_url(url_str)?;
        }
        if let Some(ref key) = self.api_key {
            if key.len() > 500 {
                return Err("API key exceeds 500 characters");
            }
        }
        if let Some(ref model) = self.model_name {
            if model.len() > 500 {
                return Err("Model name exceeds 500 characters");
            }
        }
        if let Some(ref prompt) = self.system_prompt {
            if prompt.len() > 2000 {
                return Err("System prompt exceeds 2000 characters");
            }
        }
        if let Some(temp) = self.temperature {
            if !(0.0..=2.0).contains(&temp) {
                return Err("Temperature must be between 0.0 and 2.0");
            }
        }
        if let Some(tokens) = self.max_tokens {
            if tokens <= 0 || tokens > 32768 {
                return Err("Max tokens must be between 1 and 32768");
            }
        }
        if let Some(turns) = self.max_history_turns {
            if !(1..=100).contains(&turns) {
                return Err("Max history turns must be between 1 and 100");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct AiConversation {
    pub id: String,
    pub owner_id: String,
    pub room_id: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct AiMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiChatRequest {
    pub prompt: String,
    pub conversation_id: Option<String>,
    pub room_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiChatResponse {
    pub conversation_id: String,
    pub message: String,
    pub model_used: String,
    pub tool_calls_executed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiConversationWithMessages {
    pub conversation: AiConversation,
    pub messages: Vec<AiMessage>,
}

// ---------------------------------------------------------------------------
// OpenAI Protocol Payload Structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<ChatCompletionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionMessage {
    pub role: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub r#type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionResponse {
    pub id: Option<String>,
    pub model: Option<String>,
    pub choices: Vec<ChatCompletionChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChoice {
    pub index: usize,
    pub message: ChatCompletionMessage,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunk {
    pub id: Option<String>,
    pub model: Option<String>,
    pub choices: Vec<ChatCompletionChunkChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionChunkChoice {
    pub index: usize,
    pub delta: ChatCompletionDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatCompletionDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallChunk>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallChunk {
    pub index: usize,
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub function: Option<FunctionCallChunk>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCallChunk {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_api_base_url() {
        let valid = UpdateAiConfigInput {
            api_base_url: Some("https://api.openai.com/v1".to_string()),
            ..Default::default()
        };
        assert!(valid.validate().is_ok());

        let invalid_scheme = UpdateAiConfigInput {
            api_base_url: Some("ftp://example.com".to_string()),
            ..Default::default()
        };
        assert_eq!(
            invalid_scheme.validate().unwrap_err(),
            "API base URL must use http or https scheme"
        );

        let ssrf_metadata = UpdateAiConfigInput {
            api_base_url: Some("http://169.254.169.254/latest/meta-data".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_metadata.validate().unwrap_err(),
            "API base URL cannot target link-local metadata addresses"
        );

        let ssrf_loopback = UpdateAiConfigInput {
            api_base_url: Some("http://127.0.0.1:8000/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_loopback.validate().unwrap_err(),
            "API base URL cannot target private or loopback IP addresses"
        );

        let ssrf_localhost = UpdateAiConfigInput {
            api_base_url: Some("http://localhost:11434/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_localhost.validate().unwrap_err(),
            "API base URL cannot target internal or local domain names"
        );

        let ssrf_internal = UpdateAiConfigInput {
            api_base_url: Some("http://service.internal/v1".to_string()),
            ..Default::default()
        };
        assert_eq!(
            ssrf_internal.validate().unwrap_err(),
            "API base URL cannot target internal or local domain names"
        );
    }

    #[test]
    fn test_validate_max_history_turns() {
        let valid = UpdateAiConfigInput {
            max_history_turns: Some(10),
            ..Default::default()
        };
        assert!(valid.validate().is_ok());

        let too_low = UpdateAiConfigInput {
            max_history_turns: Some(0),
            ..Default::default()
        };
        assert_eq!(
            too_low.validate().unwrap_err(),
            "Max history turns must be between 1 and 100"
        );

        let too_high = UpdateAiConfigInput {
            max_history_turns: Some(101),
            ..Default::default()
        };
        assert_eq!(
            too_high.validate().unwrap_err(),
            "Max history turns must be between 1 and 100"
        );
    }
}
