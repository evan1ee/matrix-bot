use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod activity;
pub mod activity_collectors;
pub mod handler;
pub mod models;
pub mod repository;

pub struct LifeModule;

impl LifeModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LifeModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for LifeModule {
    fn name(&self) -> &str {
        "Life"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "life stats".to_string(),
                usage: "!life stats".to_string(),
                description: "View life summary: age in days, % elapsed, and log summary"
                    .to_string(),
            },
            CommandHelp {
                name: "life today".to_string(),
                usage: "!life today".to_string(),
                description: "Check today's recorded status".to_string(),
            },
            CommandHelp {
                name: "life set-birth".to_string(),
                usage: "!life set-birth <YYYY-MM-DD> [expected_lifespan]".to_string(),
                description: "Set or update your birth date and lifespan expectancy".to_string(),
            },
            CommandHelp {
                name: "life log".to_string(),
                usage: "!life log <mood> [note]".to_string(),
                description: "Log today's mood (amazing, good, neutral, low, rough)".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_life_command(
            db.sqlite_pool(),
            broadcaster,
            &ctx.owner_id,
            &ctx.command,
            &ctx.raw_args,
        )
        .await
    }
}
