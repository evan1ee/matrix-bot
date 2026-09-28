use async_trait::async_trait;

use crate::application::AppServices;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};

pub mod handler;

pub struct JournalModule;

impl JournalModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for JournalModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for JournalModule {
    fn name(&self) -> &str {
        "Daily Log"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "journal".to_string(),
                usage: "!journal [date] (or !daily)".to_string(),
                description:
                    "View daily log, mood, weather, reflection, and cross-module activity summary"
                        .to_string(),
            },
            CommandHelp {
                name: "journal log".to_string(),
                usage: "!journal log <mood> [| weather] [| note]".to_string(),
                description:
                    "Log today's mood (amazing, good, neutral, low, rough), weather, and reflection"
                        .to_string(),
            },
            CommandHelp {
                name: "journal activities".to_string(),
                usage: "!journal activities [date]".to_string(),
                description: "View all auto-logged activities across modules for that day"
                    .to_string(),
            },
            CommandHelp {
                name: "journal clear".to_string(),
                usage: "!journal clear [date]".to_string(),
                description: "Clear the daily log entry for today or specified date".to_string(),
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
        handler::handle_journal_command(services, ctx).await
    }
}
