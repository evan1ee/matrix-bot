use chrono::{Duration, Utc};
use serde_json::json;
use sqlx::SqlitePool;
use tracing::{debug, warn};

use super::models::{FunctionDefinition, ToolDefinition};
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::portfolio::repository::PortfolioRepository;
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;
use crate::modules::todo::repository::TodoRepository;

pub struct ToolRegistry;

impl ToolRegistry {
    pub fn get_tool_definitions() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "get_my_tasks".to_string(),
                    description: "Fetch tasks and to-dos owned by the user, including status, priority, and due dates.".to_string(),
                    parameters: json!({
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
                },
            },
            ToolDefinition {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "get_my_reminders".to_string(),
                    description: "Fetch upcoming pending reminders scheduled for the user.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    }),
                },
            },
            ToolDefinition {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "get_my_recurring_bills".to_string(),
                    description: "Fetch recurring bills and subscriptions, including amounts, recurrence patterns, and next due dates.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    }),
                },
            },
            ToolDefinition {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "get_my_financial_summary".to_string(),
                    description: "Fetch financial summaries including expense and income totals broken down by category and currency.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    }),
                },
            },
            ToolDefinition {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "get_my_portfolio_summary".to_string(),
                    description: "Fetch investment portfolio holdings, valuations, uninvested cash balances, and unrealized P&L.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    }),
                },
            },
            ToolDefinition {
                r#type: "function".to_string(),
                function: FunctionDefinition {
                    name: "get_my_settings".to_string(),
                    description: "Fetch the user's configured timezone and default currency preference.".to_string(),
                    parameters: json!({
                        "type": "object",
                        "properties": {},
                        "required": [],
                        "additionalProperties": false
                    }),
                },
            },
        ]
    }

    pub async fn execute_tool(
        pool: &SqlitePool,
        owner_id: &str,
        name: &str,
        args_json: &str,
    ) -> Result<String, String> {
        debug!("Executing AI tool '{}' for owner '{}' with args: {}", name, owner_id, args_json);
        let parsed_args: serde_json::Value = if args_json.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(args_json).map_err(|e| format!("Invalid JSON arguments: {}", e))?
        };

        match name {
            "get_my_tasks" => {
                let completed = parsed_args.get("completed").and_then(|v| v.as_bool());
                let items = TodoRepository::list_all_user(pool, owner_id, completed, 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            "get_my_reminders" => {
                let items = ReminderRepository::list_user_pending(pool, owner_id, 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            "get_my_recurring_bills" => {
                let items = RecurringBillRepository::list_by_owner(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            "get_my_financial_summary" => {
                let from = Utc::now() - Duration::days(365);
                let to = Utc::now() + Duration::days(1);
                let summary = FinanceRepository::summary(pool, owner_id, from, to)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&summary).map_err(|e| e.to_string())
            }
            "get_my_portfolio_summary" => {
                let summary = PortfolioRepository::summary(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&summary).map_err(|e| e.to_string())
            }
            "get_my_settings" => {
                let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                serde_json::to_string(&settings).map_err(|e| e.to_string())
            }
            unknown => {
                warn!("Unknown AI tool requested: {}", unknown);
                Err(format!("Tool '{}' not found", unknown))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup_db() -> (Database, String) {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();
        crate::db::schema::run_migrations(&db.pool).await.unwrap();
        let account_id = crate::accounts::AccountRepository::resolve_or_provision(&db.pool, "@alice:mbm.one")
            .await
            .unwrap();
        (db, account_id)
    }

    #[tokio::test]
    async fn test_tool_definitions() {
        let defs = ToolRegistry::get_tool_definitions();
        assert_eq!(defs.len(), 6);
        let names: Vec<_> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"get_my_tasks"));
        assert!(names.contains(&"get_my_reminders"));
        assert!(names.contains(&"get_my_settings"));
    }

    #[tokio::test]
    async fn test_execute_get_my_tasks() {
        let (db, owner) = setup_db().await;
        use crate::modules::todo::models::TodoPriority;

        TodoRepository::add(&db.pool, "!room:a", &owner, "alice", "Task 1", TodoPriority::High, None, None)
            .await
            .unwrap();
        TodoRepository::add(&db.pool, "!room:a", &owner, "alice", "Task 2", TodoPriority::Low, None, None)
            .await
            .unwrap();

        let res = ToolRegistry::execute_tool(&db.pool, &owner, "get_my_tasks", "{}")
            .await
            .unwrap();
        assert!(res.contains("Task 1"));
        assert!(res.contains("Task 2"));
    }

    #[tokio::test]
    async fn test_execute_get_my_settings() {
        let (db, owner) = setup_db().await;

        let res = ToolRegistry::execute_tool(&db.pool, &owner, "get_my_settings", "{}")
            .await
            .unwrap();
        assert!(res.contains("timezone"));
        assert!(res.contains("currency"));
    }
}

