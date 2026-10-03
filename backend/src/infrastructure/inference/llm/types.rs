//! OpenAI chat-completions wire types, shared by every compatible provider.

use serde::{Deserialize, Serialize};

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
    pub tools: Option<Vec<serde_json::Value>>,
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

impl ToolDefinition {
    pub fn function(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        Self {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
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
    #[serde(default)]
    pub usage: Option<ChatUsage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChatUsage {
    #[serde(default)]
    pub prompt_tokens: i32,
    #[serde(default)]
    pub completion_tokens: i32,
    #[serde(default)]
    pub total_tokens: i32,
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
