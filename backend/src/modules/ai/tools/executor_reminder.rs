use chrono::{DateTime, Duration, TimeZone, Utc};
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_reminder_tool(
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
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'message' is required")?;
                let rec_str = args
                    .get("recurrence")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none");
                let recurrence = RecurrencePattern::from_str_or_default(rec_str);

                let trigger_at = if let Some(mins) = args.get("in_minutes").and_then(|v| v.as_i64())
                {
                    Utc::now() + Duration::minutes(mins)
                } else if let Some(trig_str) = args.get("trigger_at").and_then(|v| v.as_str()) {
                    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                    let tz = settings.tz();
                    if let Ok(dt) = DateTime::parse_from_rfc3339(trig_str) {
                        dt.with_timezone(&Utc)
                    } else if let Ok(naive_dt) =
                        chrono::NaiveDateTime::parse_from_str(trig_str, "%Y-%m-%dT%H:%M:%S")
                            .or_else(|_| {
                                chrono::NaiveDateTime::parse_from_str(trig_str, "%Y-%m-%d %H:%M:%S")
                            })
                    {
                        tz.from_local_datetime(&naive_dt)
                            .single()
                            .map(|dt| dt.with_timezone(&Utc))
                            .ok_or_else(|| {
                                "Ambiguous or invalid local time for trigger_at".to_string()
                            })?
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
                    crate::api::rooms::resolve_target_room(pool, owner_id, Some(r)).await
                } else if room_id.is_empty() || room_id == "web" || room_id == "web_chat" {
                    crate::api::rooms::resolve_target_room(pool, owner_id, None).await
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
                serde_json::to_string(&rem).map_err(|e| e.to_string())
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
