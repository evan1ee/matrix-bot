use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod adapter;
pub mod handler;
pub mod models;
pub mod prompts;
pub mod protocol;
pub mod repository;
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
            usage: "!mcp [token|list|revoke]".to_string(),
            description:
                "Manage Model Context Protocol (MCP) server status and personal access tokens"
                    .to_string(),
        }]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_command(ctx, db).await
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
