use crate::db::DbPool;
use chrono::{DateTime, Duration, TimeZone, Utc};

use crate::events::EventBroadcaster;
use crate::infrastructure::inference::llm::ToolDefinition;
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::reminder::service::{self, NewReminder};
use serde_json::json;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_reminders" => Some(
            async {
                let items = ReminderRepository::list_user_pending(pool, owner_id, 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            .await,
        ),

        "set_reminder" => Some(
            async {
                let message = args
                    .get("text")
                    .or_else(|| args.get("message"))
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'text' or 'message' is required")?;
                let rec_str = args
                    .get("recurrence")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none");
                let recurrence = RecurrencePattern::from_str_or_default(rec_str);
                let acct = crate::domain::accounts::AccountId::from(owner_id);
                let settings = services.settings.get_or_default(&acct).await;
                let tz = settings.tz();

                let trigger_at = if let Some(mins) = args.get("in_minutes").and_then(|v| {
                    v.as_i64()
                        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                }) {
                    Utc::now() + Duration::minutes(mins)
                } else if let Some(trig_str) = args
                    .get("trigger_at")
                    .or_else(|| args.get("datetime"))
                    .or_else(|| args.get("date"))
                    .or_else(|| args.get("time"))
                    .and_then(|v| v.as_str())
                {
                    let clean = trig_str.trim();
                    let clean_local = if clean.ends_with('Z') || clean.ends_with('z') {
                        &clean[..clean.len() - 1]
                    } else if clean.ends_with("+00:00") || clean.ends_with("-00:00") {
                        &clean[..clean.len() - 6]
                    } else {
                        clean
                    };

                    if let Ok(naive_dt) =
                        chrono::NaiveDateTime::parse_from_str(clean_local, "%Y-%m-%dT%H:%M:%S")
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(
                                    clean_local,
                                    "%Y-%m-%d %H:%M:%S",
                                )
                            })
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(clean_local, "%Y-%m-%dT%H:%M")
                            })
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(clean_local, "%Y-%m-%d %H:%M")
                            })
                    {
                        tz.from_local_datetime(&naive_dt)
                            .single()
                            .map(|dt| dt.with_timezone(&Utc))
                            .ok_or_else(|| {
                                "Ambiguous or invalid local time for trigger_at".to_string()
                            })?
                    } else if let Ok(dt) = DateTime::parse_from_rfc3339(clean) {
                        dt.with_timezone(&Utc)
                    } else if let Ok(naive_date) =
                        chrono::NaiveDate::parse_from_str(clean_local, "%Y-%m-%d")
                    {
                        naive_date
                            .and_hms_opt(9, 0, 0)
                            .and_then(|dt| tz.from_local_datetime(&dt).single())
                            .map(|dt| dt.with_timezone(&Utc))
                            .ok_or_else(|| "Invalid local date for reminder".to_string())?
                    } else {
                        return Err(format!("Invalid trigger_at timestamp: {}", trig_str));
                    }
                } else {
                    return Err(
                        "Either 'in_minutes' or 'trigger_at' must be provided for a reminder"
                            .to_string(),
                    );
                };

                let rooms = &services.rooms;
                let acct = crate::domain::accounts::AccountId::from(owner_id);
                let target_room = if let Some(r) = args.get("room").and_then(|v| v.as_str()) {
                    rooms.resolve_target_room(&acct, Some(r)).await
                } else if room_id.is_empty() || room_id == "web" || room_id == "web_chat" {
                    rooms.resolve_target_room(&acct, None).await
                } else {
                    room_id.to_string()
                };

                let sender_name = if let Ok(Some(acct)) =
                    crate::accounts::AccountRepository::find(pool, owner_id).await
                {
                    if !acct.display_name.trim().is_empty() {
                        acct.display_name
                    } else {
                        "You".to_string()
                    }
                } else if let Ok(Some(account_id)) =
                    crate::accounts::AccountRepository::find_by_mxid(pool, owner_id).await
                {
                    if let Ok(Some(acct)) =
                        crate::accounts::AccountRepository::find(pool, &account_id).await
                    {
                        if !acct.display_name.trim().is_empty() {
                            acct.display_name
                        } else {
                            "You".to_string()
                        }
                    } else {
                        "You".to_string()
                    }
                } else if let Some(localpart) =
                    owner_id.strip_prefix('@').and_then(|s| s.split(':').next())
                {
                    localpart.to_string()
                } else {
                    "You".to_string()
                };

                let rem = service::create(
                    pool,
                    broadcaster,
                    owner_id,
                    NewReminder {
                        message,
                        trigger_at,
                        room_id: &target_room,
                        owner_name: &sender_name,
                        recurrence,
                    },
                )
                .await
                .map_err(|e| e.to_string())?;

                let local_time = rem
                    .trigger_at
                    .with_timezone(&tz)
                    .format("%Y-%m-%d %H:%M")
                    .to_string();
                let mut resp = serde_json::to_value(&rem).map_err(|e| e.to_string())?;
                if let Some(obj) = resp.as_object_mut() {
                    obj.insert("time".to_string(), json!(local_time));
                    obj.insert("local_time".to_string(), json!(local_time));
                    obj.insert("timezone".to_string(), json!(settings.timezone));
                }
                serde_json::to_string(&resp).map_err(|e| e.to_string())
            }
            .await,
        ),

        "cancel_reminder" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;

                service::cancel(pool, broadcaster, owner_id, id)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&serde_json::json!({
                    "id": id,
                    "cancelled": true
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "get_my_reminders",
            "Fetch upcoming pending reminders scheduled for the user.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "set_reminder",
            "Schedule a future reminder for the user. Supports relative time ('in_minutes') or absolute time ('trigger_at') and recurring intervals.",
            json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "Reminder text or message (required)"
                    },
                    "message": {
                        "type": "string",
                        "description": "Optional alias for text"
                    },
                    "in_minutes": {
                        "type": "integer",
                        "description": "Minutes from now when reminder should trigger (e.g. 10, 60)"
                    },
                    "trigger_at": {
                        "type": "string",
                        "description": "Trigger time in user's default timezone (e.g. '2026-09-17 20:00:00' or '2026-09-17 20:00'). Will be interpreted in user's default timezone."
                    },
                    "recurrence": {
                        "type": "string",
                        "enum": ["daily", "weekly", "monthly", "yearly"],
                        "description": "Optional recurring frequency"
                    },
                    "room": {
                        "type": "string",
                        "description": "Matrix room name or room ID to send reminder to"
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "cancel_reminder",
            "Cancel a scheduled pending reminder by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the reminder to cancel (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_set_reminder_with_local_time() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        // Dated relative to the run: the service rejects a trigger time that has

        // passed, and a hard-coded year only defers the day this test expires.

        let day = (chrono::Utc::now() + chrono::Duration::days(365))
            .format("%Y-%m-%d")
            .to_string();

        let res = execute_tool(
            &crate::application::AppServices::new(&pool),
            &pool,
            None,
            "test_user",
            "!room:example.org",
            "set_reminder",
            &json!({
                "text": "Go running",
                // Comfortably in the future of any plausible test run date —
                // the service now rejects a trigger time that has passed.
                "trigger_at": format!("{day} 20:00:00")
            }),
        )
        .await
        .expect("tool execution result")
        .expect("success");

        let parsed: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert_eq!(parsed["message"], "Go running");
        assert_eq!(parsed["time"], format!("{day} 20:00"));
        assert_eq!(parsed["local_time"], format!("{day} 20:00"));
    }

    #[tokio::test]
    async fn test_set_reminder_with_z_suffix() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        // When an LLM sends a local time formatted as ISO-8601 with Z
        // Dated relative to the run: the service rejects a trigger time that has
        // passed, and a hard-coded year only defers the day this test expires.
        let day = (chrono::Utc::now() + chrono::Duration::days(365))
            .format("%Y-%m-%d")
            .to_string();
        let res = execute_tool(
            &crate::application::AppServices::new(&pool),
            &pool,
            None,
            "test_user",
            "!room:example.org",
            "set_reminder",
            &json!({
                "text": "Go running",
                "trigger_at": format!("{day}T20:00:00Z")
            }),
        )
        .await
        .expect("tool execution result")
        .expect("success");

        let parsed: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert_eq!(parsed["message"], "Go running");
        assert_eq!(parsed["time"], format!("{day} 20:00"));
        assert_eq!(parsed["local_time"], format!("{day} 20:00"));
    }

    #[tokio::test]
    async fn test_set_reminder_resolves_account_display_name() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let acct_id = crate::accounts::AccountRepository::create(
            &pool,
            "evan",
            crate::accounts::STATUS_ACTIVE,
        )
        .await
        .unwrap();

        let res = execute_tool(
            &crate::application::AppServices::new(&pool),
            &pool,
            None,
            &acct_id,
            "!room:example.org",
            "set_reminder",
            &json!({
                "text": "Have my hair cut",
                "in_minutes": 30
            }),
        )
        .await
        .expect("tool execution result")
        .expect("success");

        let parsed: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert_eq!(parsed["message"], "Have my hair cut");

        let items =
            ReminderRepository::list_user_reminders(&pool, &acct_id, Some("all"), None, 10, 0)
                .await
                .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].sender_name, "evan");
    }
}
