use serde_json::json;
use sqlx::SqlitePool;
use tracing::{debug, warn};

use super::executor_bookmarks_webhooks;
use super::executor_finance;
use super::executor_life_habits;
use super::executor_portfolio;
use super::executor_reminder;
use super::executor_todo;
use crate::events::EventBroadcaster;

pub async fn execute(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args_json: &str,
) -> Result<String, String> {
    debug!(
        "Executing AI tool '{}' for owner '{}' with args: {}",
        name, owner_id, args_json
    );
    let parsed_args: serde_json::Value = if args_json.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args_json).map_err(|e| format!("Invalid JSON arguments: {}", e))?
    };

    // 1. Try portfolio tools
    if let Some(res) =
        executor_portfolio::execute_portfolio_tool(pool, broadcaster, owner_id, name, &parsed_args)
            .await
    {
        return res;
    }

    // 2. Try finance tools
    if let Some(res) = executor_finance::execute_finance_tool(
        pool,
        broadcaster,
        owner_id,
        room_id,
        name,
        &parsed_args,
    )
    .await
    {
        return res;
    }

    // 3. Try reminder tools
    if let Some(res) = executor_reminder::execute_reminder_tool(
        pool,
        broadcaster,
        owner_id,
        room_id,
        name,
        &parsed_args,
    )
    .await
    {
        return res;
    }

    // 4. Try todo / bills / settings tools
    if let Some(res) =
        executor_todo::execute_todo_tool(pool, broadcaster, owner_id, room_id, name, &parsed_args)
            .await
    {
        return res;
    }

    // 4. Try life, habits, bills, footprints, and retrospective tools
    if let Some(res) = executor_life_habits::execute_life_habits_tool(
        pool,
        broadcaster,
        owner_id,
        room_id,
        name,
        &parsed_args,
    )
    .await
    {
        return res;
    }

    // 5. Try bookmarks and webhooks tools
    if let Some(res) = executor_bookmarks_webhooks::execute_bookmarks_webhooks_tool(
        pool,
        broadcaster,
        owner_id,
        name,
        &parsed_args,
    )
    .await
    {
        return res;
    }

    warn!("Unknown AI tool requested: {}", name);
    Err(format!("Tool '{}' not found", name))
}
