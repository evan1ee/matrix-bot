use async_trait::async_trait;

use crate::application::AppServices;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};

pub mod handler;

pub struct LifeModule;

impl Default for LifeModule {
    fn default() -> Self {
        Self::new()
    }
}

impl LifeModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for LifeModule {
    fn name(&self) -> &str {
        "Visualizing Life"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "life".to_string(),
                usage: "!life / !lifegrid".to_string(),
                description: "View life statistics, % of life lived, and mood tracking summary"
                    .to_string(),
            },
            CommandHelp {
                name: "life stats".to_string(),
                usage: "!life stats".to_string(),
                description: "View age in days, % of life lived, and log summary".to_string(),
            },
            CommandHelp {
                name: "life today".to_string(),
                usage: "!life today".to_string(),
                description: "View today's recorded status".to_string(),
            },
            CommandHelp {
                name: "life set-birth".to_string(),
                usage: "!life set-birth <YYYY-MM-DD> [expected_years]".to_string(),
                description: "Set your birth date and expected lifespan (defaults to 80 years)"
                    .to_string(),
            },
            CommandHelp {
                name: "life log".to_string(),
                usage: "!life log <mood> [note]".to_string(),
                description:
                    "Log today's mood (amazing, good, neutral, low, rough) and optional note"
                        .to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_life_command(services, &ctx.owner_id, &ctx.command, &ctx.raw_args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_metadata() {
        let module = LifeModule::new();
        assert_eq!(module.name(), "Visualizing Life");
        assert!(!module.commands().is_empty());
    }
}
