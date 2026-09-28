use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp, ModuleContext};

pub mod generator;
pub mod handler;
pub mod models;
pub mod scheduler;
pub mod ticker;

pub struct BriefingModule;

impl Default for BriefingModule {
    fn default() -> Self {
        Self::new()
    }
}

impl BriefingModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for BriefingModule {
    fn name(&self) -> &str {
        "Briefing"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "briefing".to_string(),
                usage: "!briefing / !morning".to_string(),
                description: "Generate and send your daily morning briefing snapshot immediately"
                    .to_string(),
            },
            CommandHelp {
                name: "briefing time".to_string(),
                usage: "!briefing time <HH:MM>".to_string(),
                description: "Set your daily morning briefing delivery time (e.g. 07:00)"
                    .to_string(),
            },
            CommandHelp {
                name: "briefing on/off".to_string(),
                usage: "!briefing [on|off]".to_string(),
                description: "Enable or disable automated daily morning briefings".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_briefing_command(
            &db.pool,
            &services.settings,
            &ctx.owner_id,
            &ctx.full_body,
            broadcaster,
        )
        .await
    }

    async fn spawn_background_workers(
        &self,
        ctx: &ModuleContext,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        vec![scheduler::start_briefing_scheduler(ctx.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_metadata() {
        let module = BriefingModule::new();
        assert_eq!(module.name(), "Briefing");
        assert!(!module.commands().is_empty());
    }
}
