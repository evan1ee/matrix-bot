use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::ai::models::ToolDefinition;
use crate::modules::settings::repository::SettingsRepository;
use crate::modules::todo::models::TodoPriority;
use crate::modules::todo::repository::TodoRepository;
use serde_json::json;

fn parse_due_at(due_str: &str, tz: &Tz) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(due_str) {
        Some(dt.with_timezone(&Utc))
    } else if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(due_str, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(due_str, "%Y-%m-%d %H:%M:%S"))
    {
        tz.from_local_datetime(&naive_dt)
            .single()
            .map(|dt| dt.with_timezone(&Utc))
    } else if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(due_str.trim(), "%Y-%m-%d") {
        naive_date
            .and_hms_opt(18, 0, 0)
            .and_then(|dt| tz.from_local_datetime(&dt).single())
            .map(|dt| dt.with_timezone(&Utc))
    } else {
        None
    }
}

pub async fn execute_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_tasks" => Some(
            async {
                let completed = args.get("completed").and_then(|v| v.as_bool());
                let items = TodoRepository::list_all_user(pool, owner_id, completed, 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            .await,
        ),

        "add_todo" => Some(
            async {
                let text = args
                    .get("text")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'text' is required")?;
                let priority_str = args
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .unwrap_or("medium");
                let priority = TodoPriority::from_str_or_default(priority_str);
                let due_at = if let Some(mins) = args.get("due_in_minutes").and_then(|v| v.as_i64())
                {
                    Some(Utc::now() + Duration::minutes(mins))
                } else if let Some(due_str) = args
                    .get("due_date")
                    .or_else(|| args.get("due_at"))
                    .or_else(|| args.get("due"))
                    .and_then(|v| v.as_str())
                {
                    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                    parse_due_at(due_str, &settings.tz())
                } else {
                    None
                };
                let remind_before = args.get("remind_before_minutes").and_then(|v| v.as_i64());

                let target_room = if let Some(r) = args.get("room").and_then(|v| v.as_str()) {
                    crate::db::rooms::resolve_target_room(pool, owner_id, Some(r)).await
                } else if room_id.is_empty() || room_id == "web" || room_id == "web_chat" {
                    crate::db::rooms::resolve_target_room(pool, owner_id, None).await
                } else {
                    room_id.to_string()
                };

                let item = TodoRepository::add(
                    pool,
                    &target_room,
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
            }
            .await,
        ),

        "complete_todo" => Some(
            async {
                let target_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                    id
                } else if let Some(search) = args
                    .get("text")
                    .or_else(|| args.get("text_search"))
                    .or_else(|| args.get("name"))
                    .and_then(|v| v.as_str())
                {
                    let active_tasks =
                        TodoRepository::list_all_user(pool, owner_id, Some(false), 50, 0)
                            .await
                            .map_err(|e| e.to_string())?;
                    let matched = active_tasks
                        .into_iter()
                        .find(|t| t.text.to_lowercase().contains(&search.to_lowercase()));
                    match matched {
                        Some(t) => t.id,
                        None => {
                            return Err(format!("No active task matching '{}' was found", search))
                        }
                    }
                } else {
                    return Err(
                        "Either 'id' or 'text' must be provided to complete a task".to_string()
                    );
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
            }
            .await,
        ),

        "update_todo" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let _existing = TodoRepository::find_owned(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Task #{} not found or not owned by user", id))?;

                if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                    TodoRepository::edit_text(pool, id, text)
                        .await
                        .map_err(|e| e.to_string())?;
                }

                if let Some(priority_str) = args.get("priority").and_then(|v| v.as_str()) {
                    let priority = TodoPriority::from_str_or_default(priority_str);
                    TodoRepository::set_priority(pool, id, priority)
                        .await
                        .map_err(|e| e.to_string())?;
                }

                if args.get("due_in_minutes").is_some() || args.get("due_at").is_some() {
                    let due_at =
                        if let Some(mins) = args.get("due_in_minutes").and_then(|v| v.as_i64()) {
                            Some(Utc::now() + Duration::minutes(mins))
                        } else if let Some(due_str) = args.get("due_at").and_then(|v| v.as_str()) {
                            let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                            parse_due_at(due_str, &settings.tz())
                        } else {
                            None
                        };
                    TodoRepository::set_due_date(pool, id, due_at)
                        .await
                        .map_err(|e| e.to_string())?;
                }

                let updated = TodoRepository::find_owned(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| "Failed to retrieve updated task".to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::TodoUpdated(updated.clone()));
                }

                serde_json::to_string(&updated).map_err(|e| e.to_string())
            }
            .await,
        ),

        "delete_todo" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let existing = TodoRepository::find_owned(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Task #{} not found or not owned by user", id))?;

                let ok = TodoRepository::delete(pool, id)
                    .await
                    .map_err(|e| e.to_string())?;

                if !ok {
                    return Err(format!("Failed to delete task #{}", id));
                }

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::TodoDeleted {
                        id,
                        room_id: existing.room_id.clone(),
                        owner_id: owner_id.to_string(),
                    });
                }

                serde_json::to_string(&serde_json::json!({
                    "id": id,
                    "deleted": true
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
            "get_my_tasks",
            "Fetch tasks and to-dos owned by the user, including status, priority, and due dates.",
            json!({
                "type": "object",
                "properties": {
                    "completed": {
                        "type": "boolean",
                        "description": "Optional filter: true for completed tasks, false for active tasks"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "add_todo",
            "Add a new task/to-do for the user. Supports priority and due dates.",
            json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "The task or todo description (required)"
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["low", "medium", "high", "urgent"],
                        "description": "Task priority level"
                    },
                    "due_date": {
                        "type": "string",
                        "description": "Due date/time in ISO-8601 (e.g. 2026-03-30T18:00:00Z) or YYYY-MM-DD"
                    },
                    "remind_before_minutes": {
                        "type": "integer",
                        "description": "Minutes before due_date to alert (e.g. 15, 60)"
                    },
                    "room": {
                        "type": "string",
                        "description": "Matrix room name or room ID to associate with the task"
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "complete_todo",
            "Mark a task/to-do as completed by its ID or exact text match.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the todo to mark as completed"
                    },
                    "text": {
                        "type": "string",
                        "description": "Exact or partial text match of the todo to complete"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "update_todo",
            "Update an existing task's description, priority, or due date.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the todo to update (required)"
                    },
                    "text": {
                        "type": "string",
                        "description": "New description text"
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["low", "medium", "high", "urgent"],
                        "description": "New priority level"
                    },
                    "due_date": {
                        "type": "string",
                        "description": "New due date in ISO 8601 format"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "delete_todo",
            "Permanently delete a task/to-do by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the todo to delete (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
    ]
}
