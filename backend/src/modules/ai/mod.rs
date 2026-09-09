use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod alert;
pub mod client;
pub mod handler;
pub mod models;
pub mod repository;
pub mod retrospective;
pub mod tools;

use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct AiModule {
    client_handle: Arc<RwLock<Option<Client>>>,
}

impl Default for AiModule {
    fn default() -> Self {
        Self::new(Arc::new(RwLock::new(None)))
    }
}

impl AiModule {
    pub fn new(client_handle: Arc<RwLock<Option<Client>>>) -> Self {
        Self { client_handle }
    }
}

#[async_trait]
impl BotModule for AiModule {
    fn name(&self) -> &str {
        "AI Assistant"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "ai".to_string(),
                usage: "!ai <prompt>".to_string(),
                description: "Chat with the AI assistant".to_string(),
            },
            CommandHelp {
                name: "ai reset".to_string(),
                usage: "!ai reset".to_string(),
                description: "Reset conversational memory in this room".to_string(),
            },
            CommandHelp {
                name: "ai model".to_string(),
                usage: "!ai model [name]".to_string(),
                description: "View or switch the active AI model".to_string(),
            },
            CommandHelp {
                name: "ai config".to_string(),
                usage: "!ai config".to_string(),
                description: "View AI backend endpoint and configuration".to_string(),
            },
            CommandHelp {
                name: "ai retro".to_string(),
                usage: "!ai retro".to_string(),
                description: "Generate a weekly executive retrospective".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_ai_command(
            &db.pool,
            broadcaster,
            Some(&self.client_handle),
            &ctx.room_id,
            &ctx.owner_id,
            &ctx.sender_name,
            &ctx.command,
            &ctx.raw_args,
            &ctx.full_body,
            ctx.is_direct,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advertises_its_commands() {
        let module = AiModule::default();
        assert_eq!(module.name(), "AI Assistant");
        let commands = module.commands();
        assert_eq!(commands.len(), 5);
    }
}
