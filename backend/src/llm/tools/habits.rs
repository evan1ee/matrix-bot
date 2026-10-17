use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use chrono::Utc;
use serde_json::json;

use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    _pool: &DbPool,
    owner_id: &AccountId,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    let acct = *owner_id;
    let settings = services.settings.get_or_default(&acct).await;
    let today_str = Utc::now()
        .with_timezone(&settings.tz())
        .format("%Y-%m-%d")
        .to_string();

    match name {
        "evaluate_habit_consistency" => Some(
            async {
                let habits = services
                    .habits
                    .list_with_status(&acct, &today_str)
                    .await
                    .map_err(|e| e.to_string())?;
                let report =
                    crate::domain::habits::consistency::evaluate_habit_consistency(&habits);
                serde_json::to_string(&report).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_my_habits" => Some(
            async {
                let habits = services
                    .habits
                    .list_with_status(&acct, &today_str)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&habits).map_err(|e| e.to_string())
            }
            .await,
        ),

        "log_habit" => Some(
            async {
                let (habit_id, habit_name) = if let Some(id) =
                    args.get("id").and_then(|v| v.as_i64())
                {
                    let habits = services
                        .habits
                        .list_with_status(&acct, &today_str)
                        .await
                        .map_err(|e| e.to_string())?;
                    let name = habits
                        .into_iter()
                        .find(|h| h.habit.id == id)
                        .map(|h| h.habit.name)
                        .unwrap_or_else(|| format!("Habit #{}", id));
                    (id, name)
                } else if let Some(search) = args
                    .get("name_search")
                    .or_else(|| args.get("name"))
                    .and_then(|v| v.as_str())
                {
                    let habits = services
                        .habits
                        .list_with_status(&acct, &today_str)
                        .await
                        .map_err(|e| e.to_string())?;
                    let matched = habits
                        .into_iter()
                        .find(|h| h.habit.name.to_lowercase().contains(&search.to_lowercase()));
                    match matched {
                        Some(h) => (h.habit.id, h.habit.name),
                        None => return Err(format!("No habit matching '{}' was found", search)),
                    }
                } else {
                    return Err(
                        "Either 'id', 'name_search', or 'name' is required to log a habit"
                            .to_string(),
                    );
                };

                let completed = services
                    .habits
                    .toggle_log(&acct, habit_id, &today_str, None)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&json!({
                    "habit_id": habit_id,
                    "habit_name": habit_name,
                    "date": today_str,
                    "completed": completed,
                    "action": if completed { "marked_completed" } else { "marked_incomplete" }
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        "create_habit" => Some(
            async {
                let name_val = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'name' is required")?;
                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("general");
                let frequency = args
                    .get("frequency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("daily");
                let target_days = args
                    .get("target_days_per_week")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(7)
                    .clamp(1, 7);

                let habit = services
                    .habits
                    .create_habit(
                        &acct,
                        name_val,
                        None,
                        Some(frequency),
                        Some(category),
                        Some(target_days),
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&habit).map_err(|e| e.to_string())
            }
            .await,
        ),

        "delete_habit" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                services
                    .habits
                    .delete_habit(&acct, id)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&json!({
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
            "get_my_habits",
            "Fetch all configured habits for the user, including today's completion status and current streaks.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "log_habit",
            "Record habit completion or check-in for a given date.",
            json!({
                "type": "object",
                "properties": {
                    "habit_id": {
                        "type": "integer",
                        "description": "The numeric ID of the habit to log (required)"
                    },
                    "date": {
                        "type": "string",
                        "description": "Optional date string in YYYY-MM-DD format (defaults to today)"
                    },
                    "completed": {
                        "type": "boolean",
                        "description": "Whether the habit was completed (defaults to true)"
                    },
                    "value": {
                        "type": "number",
                        "description": "Optional numeric value for measurable habits"
                    }
                },
                "required": ["habit_id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "create_habit",
            "Create a new habit tracking goal with target frequency (daily, weekly, weekdays).",
            json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name or title of the habit (required)"
                    },
                    "frequency": {
                        "type": "string",
                        "enum": ["daily", "weekly", "weekdays"],
                        "description": "Habit target frequency (defaults to daily)"
                    },
                    "target_per_period": {
                        "type": "integer",
                        "description": "Target completion count per frequency period (defaults to 1)"
                    }
                },
                "required": ["name"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "delete_habit",
            "Delete a habit and all its logged history by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the habit to delete (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "evaluate_habit_consistency",
            "Evaluate weekly habit completion consistency, streaks, and identify slipping or at-risk habits.",
            json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPool;
    use crate::domain::habits::consistency::HabitConsistencyReport;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[test]
    fn test_evaluate_habit_consistency_schema() {
        let defs = tool_definitions();
        let consistency_def = defs
            .iter()
            .find(|d| d.function.name == "evaluate_habit_consistency")
            .expect("evaluate_habit_consistency tool definition must exist");
        let props = consistency_def.function.parameters.get("properties").unwrap();
        assert!(props.as_object().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_execute_evaluate_habit_consistency() {
        let pool = setup().await;
        let alice = AccountId::from_seed("@alice_habit_consistency:test");
        crate::db::pool::seed_account(&pool, &alice).await;
        let services = crate::application::AppServices::new(&pool);

        let habit = services
            .habits
            .create_habit(&alice, "Daily Meditation", None, Some("daily"), None, Some(7))
            .await
            .unwrap();

        let settings = services.settings.get_or_default(&alice).await;
        let today = Utc::now()
            .with_timezone(&settings.tz())
            .format("%Y-%m-%d")
            .to_string();

        services
            .habits
            .toggle_log(&alice, habit.id, &today, None)
            .await
            .unwrap();

        let result = execute_tool(
            &services,
            &pool,
            &alice,
            "evaluate_habit_consistency",
            &json!({}),
        )
        .await
        .expect("evaluate_habit_consistency must be handled")
        .expect("tool execution should succeed");

        let report: HabitConsistencyReport =
            serde_json::from_str(&result).expect("valid HabitConsistencyReport JSON");

        assert_eq!(report.total_active, 1);
        assert_eq!(report.items.len(), 1);
        assert_eq!(report.items[0].habit_id, habit.id);
        assert_eq!(report.items[0].name, "Daily Meditation");
    }
}
