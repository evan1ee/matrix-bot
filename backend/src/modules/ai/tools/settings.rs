use serde_json::json;

use crate::application::AppServices;
use crate::domain::settings::{parse_currency, parse_timezone, SettingsError};
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

        // Validated the same way every other writer is: an LLM-supplied value
        // is untrusted input, not a source of truth (AGENTS.md, AI/agent
        // boundaries) — a bad timezone or currency is rejected here, in the
        // domain, rather than persisted and left to render as UTC forever.
        // `modify` reads-and-writes in one transaction, so a rejection never
        // touches the saved row.
        "update_my_settings" => Some(
            async {
                let saved = services
                    .settings
                    .modify(owner_id, |settings| {
                        if let Some(tz) = args.get("timezone").and_then(|v| v.as_str()) {
                            let parsed =
                                parse_timezone(tz).ok_or(SettingsError::InvalidTimezone)?;
                            settings.timezone = parsed.name().to_string();
                        }
                        if let Some(curr) = args.get("currency").and_then(|v| v.as_str()) {
                            settings.currency = parse_currency(curr)?;
                        }
                        Ok(())
                    })
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
