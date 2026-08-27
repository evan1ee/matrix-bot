use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp, ModuleContext};

pub mod handler;
pub mod models;
pub mod renderer;
pub mod repository;
pub mod scheduler;

pub struct WebhookModule;

impl Default for WebhookModule {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for WebhookModule {
    fn name(&self) -> &str {
        "Webhook"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "webhook".to_string(),
                usage: "!webhook list".to_string(),
                description: "List registered webhook endpoints".to_string(),
            },
            CommandHelp {
                name: "webhook create".to_string(),
                usage: "!webhook create <name> [room_id]".to_string(),
                description: "Create an inbound webhook for server alerts".to_string(),
            },
            CommandHelp {
                name: "webhook test".to_string(),
                usage: "!webhook test <id>".to_string(),
                description: "Send a test notification alert".to_string(),
            },
            CommandHelp {
                name: "webhook revoke".to_string(),
                usage: "!webhook revoke <id>".to_string(),
                description: "Revoke and delete a webhook".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_webhook_command(
            &db.pool,
            &ctx.room_id,
            &ctx.owner_id,
            &ctx.sender_name,
            &ctx.full_body,
            broadcaster,
        )
        .await
    }

    async fn spawn_background_workers(
        &self,
        ctx: &ModuleContext,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        vec![scheduler::start_webhook_scheduler(ctx.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_advertises_its_commands() {
        let module = WebhookModule::new();
        assert_eq!(module.name(), "Webhook");
        let commands = module.commands();
        assert_eq!(commands.len(), 4);
    }
}
