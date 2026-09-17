use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod adapter;
pub mod prompts;
pub mod protocol;
pub mod resources;
pub mod service;
pub mod stdio;

pub struct McpModule;

impl McpModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for McpModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for McpModule {
    fn name(&self) -> &str {
        "MCP"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![CommandHelp {
            name: "mcp".to_string(),
            usage: "!mcp".to_string(),
            description: "Show Model Context Protocol (MCP) server status and configuration".to_string(),
        }]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        _db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        if ctx.command != "mcp" {
            return None;
        }

        let tools = adapter::get_mcp_tools();
        let res = resources::list_resources();
        let p = prompts::list_prompts();

        let html = format!(
            "<b>Model Context Protocol (MCP) Server Active</b><br/>\
             <ul>\
               <li><b>Exposed Tools:</b> {} tools (todos, finance, portfolio, habits, bookmarks, life)</li>\
               <li><b>Readable Resources:</b> {} URIs (<code>matrixbot://...</code>)</li>\
               <li><b>Prompt Templates:</b> {} templates</li>\
             </ul>\
             <p>Connect your desktop AI assistant (Claude Desktop, Cursor) via STDIO (<code>matrix-bot mcp</code>) or over Streamable HTTP/SSE (<code>/api/v1/mcp/sse</code>).</p>",
            tools.len(),
            res.len(),
            p.len()
        );

        Some(html)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_module_metadata() {
        let module = McpModule::new();
        assert_eq!(module.name(), "MCP");
        assert_eq!(module.commands().len(), 1);
        assert_eq!(module.commands()[0].name, "mcp");
    }
}
