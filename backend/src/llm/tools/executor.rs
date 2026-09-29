use crate::db::DbPool;
use serde_json::json;
use tracing::{debug, warn};

use super::bills;
use super::bookmarks;
use super::finance;
use super::habits;
use super::life;
use super::portfolio;
use super::reminder;
use super::settings;
use super::todo;
use super::webhooks;
use crate::events::EventBroadcaster;

pub async fn execute(
    services: &crate::application::AppServices,
    pool: &DbPool,
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

    if name == "web_search" {
        let query = parsed_args
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .trim();
        if query.is_empty() {
            return Err("Missing 'query' parameter for web_search".to_string());
        }
        return crate::llm::search::execute_web_search(query).await;
    }

    if name == "x_search" {
        return Ok("X/Twitter search processed.".to_string());
    }

    // 1. Try portfolio tools
    if let Some(res) =
        portfolio::execute_tool(services, pool, broadcaster, owner_id, name, &parsed_args).await
    {
        return res;
    }

    // 2. Try finance tools
    if let Some(res) = finance::execute_tool(
        services,
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
    if let Some(res) =
        reminder::execute_tool(services, broadcaster, owner_id, room_id, name, &parsed_args).await
    {
        return res;
    }

    // 4. Try todo tools
    if let Some(res) = todo::execute_tool(
        services,
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

    // 5. Try recurring bills tools
    if let Some(res) = bills::execute_tool(
        services,
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

    // 6. Try habits tools
    if let Some(res) =
        habits::execute_tool(services, pool, broadcaster, owner_id, name, &parsed_args).await
    {
        return res;
    }

    // 7. Try life, daily log, retrospective, and footprint tools
    if let Some(res) =
        life::execute_tool(services, pool, broadcaster, owner_id, name, &parsed_args).await
    {
        return res;
    }

    // 8. Try bookmark tools
    if let Some(res) =
        bookmarks::execute_tool(pool, broadcaster, owner_id, name, &parsed_args).await
    {
        return res;
    }

    // 9. Try webhook tools
    if let Some(res) = webhooks::execute_tool(services, owner_id, name, &parsed_args).await {
        return res;
    }

    // 10. Try settings tools
    if let Some(res) = settings::execute_tool(services, owner_id, name, &parsed_args).await {
        return res;
    }

    warn!("Unknown AI tool requested: {}", name);
    Err(format!("Tool '{}' not found", name))
}
