use serde_json::json;

use crate::application::AppServices;
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_settings" => Some(
            async {
                let settings = services.settings.get_or_default(owner_id).await;
                serde_json::to_string(&settings).map_err(|e| e.to_string())
            }
            .await,
        ),

        "update_my_settings" => Some(
            async {
                let mut settings = services.settings.get_or_default(owner_id).await;
                if let Some(tz) = args.get("timezone").and_then(|v| v.as_str()) {
                    settings.timezone = tz.to_string();
                }
                if let Some(curr) = args.get("currency").and_then(|v| v.as_str()) {
                    settings.currency = curr.to_uppercase();
                }

                let saved = services
                    .settings
                    .save(owner_id, settings)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&saved).map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "get_my_settings",
            "Fetch the user's active configuration settings, including timezone, display currency, and dev room.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "update_my_settings",
            "Update user configuration preferences such as timezone, display currency, or default dev room.",
            json!({
                "type": "object",
                "properties": {
                    "timezone": {
                        "type": "string",
                        "description": "IANA timezone string (e.g. America/New_York, Asia/Taipei, UTC)"
                    },
                    "currency": {
                        "type": "string",
                        "description": "3-letter currency code (e.g. USD, EUR, TWD)"
                    },
                    "dev_room_id": {
                        "type": "string",
                        "description": "Matrix room ID for development / testing"
                    },
                    "max_history_turns": {
                        "type": "integer",
                        "description": "Maximum chat history turns to retain in context (1-100)"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
    ]
}
