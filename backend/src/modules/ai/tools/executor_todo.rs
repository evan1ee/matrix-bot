use chrono::{DateTime, Duration, Utc};
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;
use crate::modules::todo::models::TodoPriority;
use crate::modules::todo::repository::TodoRepository;

pub async fn execute_todo_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_tasks" => Some(async {
            let completed = args.get("completed").and_then(|v| v.as_bool());
            let items = TodoRepository::list_all_user(pool, owner_id, completed, 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }.await),

        "get_my_reminders" => Some(async {
            let items = ReminderRepository::list_user_pending(pool, owner_id, 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }.await),

        "get_my_recurring_bills" => Some(async {
            let items = RecurringBillRepository::list_by_owner(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }.await),

        "get_my_settings" => Some(async {
            let settings = SettingsRepository::get_or_default(pool, owner_id).await;
            serde_json::to_string(&settings).map_err(|e| e.to_string())
        }.await),

        "add_todo" => Some(async {
            let text = args.get("text").and_then(|v| v.as_str()).ok_or("Field 'text' is required")?;
            let priority_str = args.get("priority").and_then(|v| v.as_str()).unwrap_or("medium");
            let priority = TodoPriority::from_str_or_default(priority_str);
            let due_at = if let Some(mins) = args.get("due_in_minutes").and_then(|v| v.as_i64()) {
                Some(Utc::now() + Duration::minutes(mins))
            } else if let Some(due_str) = args.get("due_at").and_then(|v| v.as_str()) {
                DateTime::parse_from_rfc3339(due_str).map(|dt| dt.with_timezone(&Utc)).ok()
            } else {
                None
            };
            let remind_before = args.get("remind_before_minutes").and_then(|v| v.as_i64());

            let item = TodoRepository::add(
                pool,
                room_id,
                owner_id,
                "AI Assistant",
                text,
                priority,
                due_at,
                remind_before,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TodoCreated(item.clone()));
            }
            serde_json::to_string(&item).map_err(|e| e.to_string())
        }.await),

        "complete_todo" => Some(async {
            let target_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                id
            } else if let Some(search) = args.get("text_search").and_then(|v| v.as_str()) {
                let active_tasks = TodoRepository::list_all_user(pool, owner_id, Some(false), 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                let matched = active_tasks.into_iter().find(|t| t.text.to_lowercase().contains(&search.to_lowercase()));
                match matched {
                    Some(t) => t.id,
                    None => return Err(format!("No active task matching '{}' was found", search)),
                }
            } else {
                return Err("Either 'id' or 'text_search' must be provided to complete a task".to_string());
            };

            let item = TodoRepository::find_owned(pool, target_id, owner_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task #{} not found or not owned by user", target_id))?;

            let updated = TodoRepository::set_completed(pool, item.id, true)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Failed to complete task".to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TodoUpdated(updated.clone()));
            }
            serde_json::to_string(&updated).map_err(|e| e.to_string())
        }.await),

        "set_reminder" => Some(async {
            let message = args.get("message").and_then(|v| v.as_str()).ok_or("Field 'message' is required")?;
            let rec_str = args.get("recurrence").and_then(|v| v.as_str()).unwrap_or("none");
            let recurrence = RecurrencePattern::from_str_or_default(rec_str);

            let trigger_at = if let Some(mins) = args.get("in_minutes").and_then(|v| v.as_i64()) {
                Utc::now() + Duration::minutes(mins)
            } else if let Some(trig_str) = args.get("trigger_at").and_then(|v| v.as_str()) {
                DateTime::parse_from_rfc3339(trig_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| format!("Invalid trigger_at timestamp: {}", e))?
            } else {
                return Err("Either 'in_minutes' or 'trigger_at' must be provided for a reminder".to_string());
            };

            let rem = ReminderRepository::create(
                pool,
                room_id,
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
        }.await),

        _ => None,
    }
}
