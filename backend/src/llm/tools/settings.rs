use crate::domain::accounts::AccountId;
use serde_json::json;

use crate::application::AppServices;
use crate::domain::settings::{
    parse_currency, parse_max_history_turns, parse_timezone, SettingsError,
};
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &AccountId,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_settings" => Some(
            async {
                let acct = *owner_id;
                let settings = services.settings.get_or_default(&acct).await;
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
                let acct = *owner_id;
                let saved = services
                    .settings
                    .modify(&acct, |settings| {
                        if let Some(tz) = args.get("timezone").and_then(|v| v.as_str()) {
                            let parsed =
                                parse_timezone(tz).ok_or(SettingsError::InvalidTimezone)?;
                            settings.timezone = parsed.name().to_string();
                        }
                        if let Some(curr) = args.get("currency").and_then(|v| v.as_str()) {
                            settings.currency = parse_currency(curr)?;
                        }
                        if let Some(r) = args.get("dev_room_id") {
                            settings.dev_room_id = if r.is_null() {
                                None
                            } else if let Some(s) = r.as_str() {
                                let trimmed = s.trim();
                                if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("none") {
                                    None
                                } else {
                                    Some(trimmed.to_string())
                                }
                            } else {
                                None
                            };
                        }
                        if let Some(turns) = args.get("max_history_turns").and_then(|v| v.as_i64())
                        {
                            settings.max_history_turns = parse_max_history_turns(turns)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_update_my_settings_all_fields() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let owner =
            crate::infrastructure::postgres::repositories::identities::resolve_or_provision(
                &pool,
                &crate::domain::channels::PlatformIdentity::matrix("@settings_tester:mbm.one")
                    .unwrap(),
            )
            .await
            .unwrap();

        let services = AppServices::new(&pool);

        // Initially default
        let initial = services.settings.get_or_default(&owner).await;
        assert_eq!(initial.max_history_turns, 10);
        assert_eq!(initial.dev_room_id, None);

        // Update dev_room_id and max_history_turns
        let args = json!({
            "timezone": "Asia/Tokyo",
            "currency": "JPY",
            "dev_room_id": "!dev:test",
            "max_history_turns": 25
        });

        let res = execute_tool(&services, &owner, "update_my_settings", &args)
            .await
            .expect("tool exists")
            .expect("update succeeds");

        assert!(res.contains("Asia/Tokyo"));
        assert!(res.contains("JPY"));
        assert!(res.contains("!dev:test"));
        assert!(res.contains("25"));

        let updated = services.settings.get_or_default(&owner).await;
        assert_eq!(updated.timezone, "Asia/Tokyo");
        assert_eq!(updated.currency, "JPY");
        assert_eq!(updated.dev_room_id.as_deref(), Some("!dev:test"));
        assert_eq!(updated.max_history_turns, 25);

        // Update dev_room_id to "none" (cleared to None)
        let clear_args = json!({
            "dev_room_id": "none"
        });
        execute_tool(&services, &owner, "update_my_settings", &clear_args)
            .await
            .unwrap()
            .unwrap();

        let cleared = services.settings.get_or_default(&owner).await;
        assert_eq!(cleared.dev_room_id, None);
        // max_history_turns preserved
        assert_eq!(cleared.max_history_turns, 25);

        // Out-of-range max_history_turns returns error
        let bad_args = json!({
            "max_history_turns": 200
        });
        let err = execute_tool(&services, &owner, "update_my_settings", &bad_args)
            .await
            .unwrap();
        assert!(err.is_err());
    }
}
