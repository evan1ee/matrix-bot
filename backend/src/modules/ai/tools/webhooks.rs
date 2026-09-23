use crate::db::DbPool;
use serde_json::json;

use crate::modules::webhook::repository::WebhookRepository;
use crate::services::llm::ToolDefinition;

pub async fn execute_tool(
    pool: &DbPool,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_webhooks" => Some(
            async {
                let hooks = WebhookRepository::list_by_owner(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&hooks).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_webhook_logs" => Some(
            async {
                let webhook_id = args
                    .get("webhook_id")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'webhook_id' is required")?;
                let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10);
                let logs =
                    WebhookRepository::list_outbox_by_webhook(pool, webhook_id, owner_id, limit, 0)
                        .await
                        .map_err(|e| e.to_string())?;
                serde_json::to_string(&logs).map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "get_my_webhooks",
            "List all configured inbound webhooks, their destination rooms, and delivery statistics.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_webhook_logs",
            "Fetch recent delivery logs and incoming payloads for a specific webhook ID.",
            json!({
                "type": "object",
                "properties": {
                    "webhook_id": {
                        "type": "string",
                        "description": "Webhook ID string (e.g. 'whk_...')"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of logs to return (default 10)"
                    }
                },
                "required": ["webhook_id"],
                "additionalProperties": false
            }),
        ),
    ]
}
