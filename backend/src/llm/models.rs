use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub use crate::domain::llm::validate_ai_base_url;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    pub max_history_turns: Option<i64>,
    pub updated_at: String,
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

impl From<crate::domain::llm::AiUserConfig> for AiUserConfig {
    fn from(c: crate::domain::llm::AiUserConfig) -> Self {
        Self {
            owner_id: c.owner_id,
            api_base_url: c.api_base_url,
            api_key: c.api_key,
            model_name: c.model_name,
            system_prompt: c.system_prompt,
            temperature: c.temperature,
            max_tokens: c.max_tokens,
            max_history_turns: c.max_history_turns,
            updated_at: c.updated_at,
            has_api_key: c.has_api_key,
        }
    }
}

impl From<AiUserConfig> for crate::domain::llm::AiUserConfig {
    fn from(c: AiUserConfig) -> Self {
        Self {
            owner_id: c.owner_id,
            api_base_url: c.api_base_url,
            api_key: c.api_key,
            model_name: c.model_name,
            system_prompt: c.system_prompt,
            temperature: c.temperature,
            max_tokens: c.max_tokens,
            max_history_turns: c.max_history_turns,
            updated_at: c.updated_at,
            has_api_key: c.has_api_key,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct VerifyAiKeyInput {
    pub api_base_url: Option<String>,
    pub api_key: Option<String>,
    pub model_name: Option<String>,
}

impl VerifyAiKeyInput {
    pub fn validate(&self, allow_private: bool) -> Result<(), &'static str> {
        let domain_input = crate::domain::llm::VerifyAiKeyInput {
            api_base_url: self.api_base_url.clone(),
            api_key: self.api_key.clone(),
            model_name: self.model_name.clone(),
        };
        domain_input.validate(allow_private)
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
    pub fn validate(&self, allow_private: bool) -> Result<(), &'static str> {
        let domain_input: crate::domain::llm::UpdateAiConfigInput = self.clone().into();
        domain_input.validate(allow_private)
    }
}

impl From<UpdateAiConfigInput> for crate::domain::llm::UpdateAiConfigInput {
    fn from(i: UpdateAiConfigInput) -> Self {
        Self {
            api_base_url: i.api_base_url,
            api_key: i.api_key,
            model_name: i.model_name,
            system_prompt: i.system_prompt,
            temperature: i.temperature,
            max_tokens: i.max_tokens,
            max_history_turns: i.max_history_turns,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiConversation {
    pub id: String,
    pub owner_id: String,
    pub room_id: String,
    pub created_at: String,
    pub updated_at: String,
}

impl From<crate::domain::llm::AiConversation> for AiConversation {
    fn from(c: crate::domain::llm::AiConversation) -> Self {
        Self {
            id: c.id,
            owner_id: c.owner_id,
            room_id: c.room_id,
            created_at: c.created_at,
            updated_at: c.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AiMessage {
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub created_at: String,
}

impl From<crate::domain::llm::AiMessage> for AiMessage {
    fn from(m: crate::domain::llm::AiMessage) -> Self {
        Self {
            id: m.id,
            conversation_id: m.conversation_id,
            role: m.role,
            content: m.content,
            tool_call_id: m.tool_call_id,
            tool_name: m.tool_name,
            created_at: m.created_at,
        }
    }
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
