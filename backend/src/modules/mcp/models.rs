use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct McpToken {
    pub id: String,
    pub account_id: String,
    pub name: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl McpToken {
    /// Whether this token has expired.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at.is_some_and(|exp| now > exp)
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateMcpTokenRequest {
    pub name: String,
    pub expires_in_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CreateMcpTokenResponse {
    pub id: String,
    pub name: String,
    pub token: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub claude_config: serde_json::Value,
    pub cursor_config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct McpTokenSummary {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl From<McpToken> for McpTokenSummary {
    fn from(t: McpToken) -> Self {
        Self {
            id: t.id,
            name: t.name,
            prefix: t.prefix,
            created_at: t.created_at,
            last_used_at: t.last_used_at,
            expires_at: t.expires_at,
        }
    }
}

/// Builds sample Claude Desktop config JSON with this token and endpoint URL.
pub fn build_claude_config(endpoint_url: &str, token: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "matrix-bot": {
                "url": endpoint_url,
                "headers": {
                    "Authorization": format!("Bearer {}", token)
                }
            }
        }
    })
}

/// Builds sample Cursor config JSON with this token and endpoint URL.
pub fn build_cursor_config(endpoint_url: &str, token: &str) -> serde_json::Value {
    serde_json::json!({
        "mcpServers": {
            "matrix-bot": {
                "url": endpoint_url,
                "headers": {
                    "Authorization": format!("Bearer {}", token)
                }
            }
        }
    })
}
