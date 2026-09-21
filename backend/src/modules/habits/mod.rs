use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod models;
pub mod repository;

pub struct HabitsModule;

impl HabitsModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for HabitsModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for HabitsModule {
    fn name(&self) -> &str {
        "Habits"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "habit list".to_string(),
                usage: "!habit [list]".to_string(),
                description: "List today's habits, completion status, and streaks".to_string(),
            },
            CommandHelp {
                name: "habit done".to_string(),
                usage: "!habit done <name or id>".to_string(),
                description: "Check off a habit for today".to_string(),
            },
            CommandHelp {
                name: "habit undo".to_string(),
                usage: "!habit undo <name or id>".to_string(),
                description: "Uncheck a habit for today".to_string(),
            },
            CommandHelp {
                name: "habit add".to_string(),
                usage: "!habit add <name>".to_string(),
                description: "Create a new daily habit".to_string(),
            },
            CommandHelp {
                name: "habit streak".to_string(),
                usage: "!habit streak".to_string(),
                description: "View streaks leaderboard".to_string(),
            },
            CommandHelp {
                name: "habit delete".to_string(),
                usage: "!habit delete <id>".to_string(),
                description: "Delete a habit by ID".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_habit_command(
            &db.pool,
            broadcaster,
            &ctx.owner_id,
            &ctx.command,
            &ctx.raw_args,
        )
        .await
    }
}
