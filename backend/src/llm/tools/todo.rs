use crate::db::DbPool;
use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;

use crate::domain::accounts::AccountId;
use crate::domain::reminders::RecurrencePattern;
use crate::domain::tags::{parse_tags, Tag};
use crate::domain::todos::{NewTodo, TodoItem, TodoPatch, TodoPriority};
use crate::infrastructure::inference::llm::ToolDefinition;
use serde_json::json;

/// A task as `get_my_tasks` reports it: the item plus checklist progress.
#[derive(serde::Serialize)]
struct TaskView {
    #[serde(flatten)]
    item: TodoItem,
    steps_done: i64,
    steps_total: i64,
}

/// Strict: an unknown value is an error for the model, never a silent default.
fn parse_recurrence(raw: &str) -> Result<RecurrencePattern, String> {
    match raw {
        "none" => Ok(RecurrencePattern::None),
        "daily" => Ok(RecurrencePattern::Daily),
        "weekly" => Ok(RecurrencePattern::Weekly),
        "monthly" => Ok(RecurrencePattern::Monthly),
        "yearly" => Ok(RecurrencePattern::Yearly),
        other => Err(format!(
            "Invalid recurrence '{other}': use none, daily, weekly, monthly or yearly"
        )),
    }
}

fn arg_recurrence(args: &serde_json::Value) -> Result<Option<RecurrencePattern>, String> {
    match args.get("recurrence") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .ok_or_else(|| "Field 'recurrence' must be a string".to_string())
            .and_then(parse_recurrence)
            .map(Some),
    }
}

fn arg_tags(args: &serde_json::Value) -> Result<Option<Vec<Tag>>, String> {
    match args.get("tags") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(v) => {
            let raw: Vec<String> = serde_json::from_value(v.clone())
                .map_err(|_| "Field 'tags' must be an array of strings".to_string())?;
            parse_tags(&raw).map(Some).map_err(|e| e.to_string())
        }
    }
}

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
    services: &crate::application::AppServices,
    _pool: &DbPool,
    owner_id: &AccountId,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_tasks" => Some(
            async {
                let acct = *owner_id;
                let completed = args.get("completed").and_then(|v| v.as_bool());
                let tag = args
                    .get("tag")
                    .and_then(|v| v.as_str())
                    .map(Tag::parse)
                    .transpose()
                    .map_err(|e| e.to_string())?;
                let items = services
                    .todos
                    .list(&acct, None, completed, tag.as_ref(), 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                let ids: Vec<i64> = items.iter().map(|t| t.id).collect();
                let progress = services
                    .todos
                    .step_progress(&acct, &ids)
                    .await
                    .map_err(|e| e.to_string())?;
                let views: Vec<TaskView> = items
                    .into_iter()
                    .map(|item| {
                        let (steps_done, steps_total) =
                            progress.get(&item.id).copied().unwrap_or((0, 0));
                        TaskView {
                            item,
                            steps_done,
                            steps_total,
                        }
                    })
                    .collect();
                serde_json::to_string(&views).map_err(|e| e.to_string())
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
                    let acct = *owner_id;
                    let settings = services.settings.get_or_default(&acct).await;
                    parse_due_at(due_str, &settings.tz())
                } else {
                    None
                };
                let remind_before = args.get("remind_before_minutes").and_then(|v| v.as_i64());
                let tags = arg_tags(args)?.unwrap_or_default();
                let recurrence = arg_recurrence(args)?.unwrap_or_default();

                let rooms = &services.rooms;
                let acct = *owner_id;
                let target_room = if let Some(r) = args.get("room").and_then(|v| v.as_str()) {
                    rooms.resolve_target_room(&acct, Some(r)).await
                } else if matches!(
                    crate::domain::rooms::RoomTarget::parse(Some(room_id)),
                    crate::domain::rooms::RoomTarget::UserDefault
                ) {
                    rooms.resolve_target_room(&acct, None).await
                } else {
                    room_id.to_string()
                };

                let acct = *owner_id;
                let (item, _) = services
                    .todos
                    .create(
                        &acct,
                        NewTodo {
                            room_id: target_room,
                            sender_name: "AI Assistant".to_string(),
                            text: text.to_string(),
                            priority,
                            due_at,
                            remind_before_minutes: remind_before,
                            remind_room_id: None,
                            tags,
                            recurrence,
                            recurrence_anchor: None,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&item).map_err(|e| e.to_string())
            }
            .await,
        ),

        "complete_todo" => Some(
            async {
                let acct = *owner_id;
                let target_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                    id
                } else if let Some(search) = args
                    .get("text")
                    .or_else(|| args.get("text_search"))
                    .or_else(|| args.get("name"))
                    .and_then(|v| v.as_str())
                {
                    let active_tasks = services
                        .todos
                        .list(&acct, None, Some(false), None, 50, 0)
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

                let outcome = services
                    .todos
                    .set_completed(&acct, target_id, true)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&json!({ "todo": outcome.todo, "next": outcome.next }))
                    .map_err(|e| e.to_string())
            }
            .await,
        ),

        "update_todo" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;

                let text = args.get("text").and_then(|v| v.as_str());
                let priority = args
                    .get("priority")
                    .and_then(|v| v.as_str())
                    .map(TodoPriority::from_str_or_default);

                let due_at = if args.get("due_in_minutes").is_some() || args.get("due_at").is_some()
                {
                    let due_at =
                        if let Some(mins) = args.get("due_in_minutes").and_then(|v| v.as_i64()) {
                            Some(Utc::now() + Duration::minutes(mins))
                        } else if let Some(due_str) = args.get("due_at").and_then(|v| v.as_str()) {
                            let acct = *owner_id;
                            let settings = services.settings.get_or_default(&acct).await;
                            parse_due_at(due_str, &settings.tz())
                        } else {
                            None
                        };
                    Some(due_at)
                } else {
                    None
                };

                let tags = arg_tags(args)?;
                let recurrence = arg_recurrence(args)?;

                let acct = *owner_id;
                let (updated, _) = services
                    .todos
                    .update(
                        &acct,
                        id,
                        TodoPatch {
                            text: text.map(ToString::to_string),
                            priority,
                            due_at,
                            tags,
                            recurrence,
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;

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

                let acct = *owner_id;
                services
                    .todos
                    .delete(&acct, id)
                    .await
                    .map_err(|e| e.to_string())?;

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
                    },
                    "tag": {
                        "type": "string",
                        "description": "Optional filter: only tasks carrying this tag"
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
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "maxItems": 10,
                        "description": "Tags (letters, digits, - and _ only)"
                    },
                    "recurrence": {
                        "type": "string",
                        "enum": ["none", "daily", "weekly", "monthly", "yearly"],
                        "description": "Repeat schedule; completing a repeating task creates the next one (needs a due date)"
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
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" },
                        "maxItems": 10,
                        "description": "Replaces the task's whole tag set (letters, digits, - and _ only)"
                    },
                    "recurrence": {
                        "type": "string",
                        "enum": ["none", "daily", "weekly", "monthly", "yearly"],
                        "description": "New repeat schedule"
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::domain::reminders::ReminderSource;
    use crate::domain::todos::MAX_TEXT_LEN;
    use crate::infrastructure::postgres::repositories::reminders as reminders_repo;

    async fn setup_db() -> (Database, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let db = Database { pool };
        let account_id = crate::application::AppServices::new(&db.pool)
            .identity
            .provision_matrix_for_test("@alice:mbm.one")
            .await;
        (db, account_id)
    }

    /// The AI path used to call the repository directly and skip validation
    /// entirely — this is the length limit every other transport enforces.
    #[tokio::test]
    async fn test_the_ai_tool_enforces_the_text_limit() {
        let (db, owner) = setup_db().await;
        let over_limit = "x".repeat(MAX_TEXT_LEN + 1);
        let args = json!({ "text": over_limit });

        let result = execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "add_todo",
            &args,
        )
        .await;
        assert!(matches!(result, Some(Err(_))));
    }

    /// The bug this whole conversion exists to fix: `add_todo` never reconciled
    /// a lead-time reminder, so a due date plus `remind_before_minutes` silently
    /// scheduled nothing.
    #[tokio::test]
    async fn test_the_ai_tool_schedules_a_due_reminder() {
        let (db, owner) = setup_db().await;
        let due = (Utc::now() + Duration::hours(4)).to_rfc3339();
        let args = json!({
            "text": "Ship it",
            "due_at": due,
            "remind_before_minutes": 60,
        });

        let result = execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "add_todo",
            &args,
        )
        .await
        .unwrap()
        .unwrap();
        let todo: serde_json::Value = serde_json::from_str(&result).unwrap();
        let todo_id = todo["id"].as_i64().unwrap();

        let pending =
            reminders_repo::find_pending_for_source(&db.pool, ReminderSource::Todo, todo_id)
                .await
                .unwrap();
        assert!(pending.is_some(), "a due reminder must be scheduled");
    }
}
