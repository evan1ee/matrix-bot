use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::ai::models::ToolDefinition;
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;
use serde_json::json;

pub async fn execute_tool(
    pool: &SqlitePool,
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

                let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                let tz = settings.tz();

                let trigger_at = if let Some(mins) = args
                    .get("in_minutes")
                    .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                {
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
                                chrono::NaiveDateTime::parse_from_str(clean_local, "%Y-%m-%d %H:%M:%S")
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

                let target_room = if let Some(r) = args.get("room").and_then(|v| v.as_str()) {
                    crate::db::rooms::resolve_target_room(pool, owner_id, Some(r)).await
                } else if room_id.is_empty() || room_id == "web" || room_id == "web_chat" {
                    crate::db::rooms::resolve_target_room(pool, owner_id, None).await
                } else {
                    room_id.to_string()
                };

                let rem = ReminderRepository::create(
                    pool,
                    &target_room,
                    owner_id,
                    "AI Assistant",
                    message,
                    trigger_at,
                    recurrence,
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::ReminderCreated(rem.clone()));
                }

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
                let rem = ReminderRepository::find_owned(pool, owner_id, id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Reminder #{} not found or not owned by user", id))?;

                let ok = ReminderRepository::cancel(pool, owner_id, id)
                    .await
                    .map_err(|e| e.to_string())?;

                if !ok {
                    return Err(format!("Reminder #{} could not be cancelled", id));
                }

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::ReminderCancelled {
                        id,
                        room_id: rem.room_id.clone(),
                        owner_id: owner_id.to_string(),
                    });
                }

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

    #[sqlx::test]
    async fn test_set_reminder_with_local_time(pool: SqlitePool) {
        crate::db::schema::run_migrations(&pool).await.unwrap();

        let res = execute_tool(
            &pool,
            None,
            "test_user",
            "!room:example.org",
            "set_reminder",
            &json!({
                "text": "Go running",
                "trigger_at": "2026-09-17 20:00:00"
            }),
        )
        .await
        .expect("tool execution result")
        .expect("success");

        let parsed: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert_eq!(parsed["message"], "Go running");
        assert_eq!(parsed["time"], "2026-09-17 20:00");
        assert_eq!(parsed["local_time"], "2026-09-17 20:00");
    }

    #[sqlx::test]
    async fn test_set_reminder_with_z_suffix(pool: SqlitePool) {
        crate::db::schema::run_migrations(&pool).await.unwrap();

        // When an LLM sends a local time formatted as ISO-8601 with Z (e.g. 2026-09-17T20:00:00Z)
        let res = execute_tool(
            &pool,
            None,
            "test_user",
            "!room:example.org",
            "set_reminder",
            &json!({
                "text": "Go running",
                "trigger_at": "2026-09-17T20:00:00Z"
            }),
        )
        .await
        .expect("tool execution result")
        .expect("success");

        let parsed: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert_eq!(parsed["message"], "Go running");
        assert_eq!(parsed["time"], "2026-09-17 20:00");
        assert_eq!(parsed["local_time"], "2026-09-17 20:00");
    }
}

