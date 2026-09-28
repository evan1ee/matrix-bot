use crate::db::DbPool;
use tracing::{debug, warn};

use super::protocol::{CallToolParams, CallToolResult, McpTool, ToolContent};
use crate::events::EventBroadcaster;
use crate::modules::ai::tools::definitions::all_tool_definitions;
use crate::modules::ai::tools::executor;

/// Translates all unified AI function calling tool definitions into standard MCP tool schemas.
pub fn get_mcp_tools() -> Vec<McpTool> {
    all_tool_definitions()
        .into_iter()
        .map(|def| McpTool {
            name: def.function.name,
            description: Some(def.function.description),
            input_schema: def.function.parameters,
        })
        .collect()
}

/// Executes an MCP tool call by delegating to the unified AI tool executor.
pub async fn call_mcp_tool(
    services: &crate::application::AppServices,
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    params: CallToolParams,
) -> CallToolResult {
    debug!(
        "MCP tool call '{}' requested by owner '{}'",
        params.name, owner_id
    );
    let args_str = params
        .arguments
        .as_ref()
        .map(|v| v.to_string())
        .unwrap_or_else(|| "{}".to_string());

    // Use "mcp" as the synthetic room identifier so actions are tagged with an MCP origin
    match executor::execute(
        services,
        pool,
        broadcaster,
        owner_id,
        "mcp",
        &params.name,
        &args_str,
    )
    .await
    {
        Ok(output) => CallToolResult {
            content: vec![ToolContent::Text { text: output }],
            is_error: Some(false),
        },
        Err(err) => {
            warn!("MCP tool call '{}' failed: {}", params.name, err);
            CallToolResult {
                content: vec![ToolContent::Text { text: err }],
                is_error: Some(true),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tools_conversion() {
        let tools = get_mcp_tools();
        assert!(!tools.is_empty(), "Expected non-empty MCP tool list");
        assert!(tools.iter().any(|t| t.name == "add_todo"));
        assert!(tools.iter().any(|t| t.name == "get_my_tasks"));
        assert!(tools.iter().any(|t| t.name == "record_finance_entry"));
    }
}
