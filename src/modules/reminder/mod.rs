use async_trait::async_trait;

use crate::db::Database;
use crate::modules::{BotModule, CommandContext, CommandHelp, ModuleContext};

pub mod handler;
pub mod models;
pub mod repository;
pub mod scheduler;

pub struct ReminderModule;

impl ReminderModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for ReminderModule {
    fn name(&self) -> &str {
        "Reminder"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "remind".to_string(),
                usage: "!remind [daily|weekly] <time> <message>".to_string(),
                description: "Set a one-time or recurring timer (e.g., !remind 10s check oven)".to_string(),
            },
            CommandHelp {
                name: "remind list".to_string(),
                usage: "!remind list".to_string(),
                description: "List your active pending reminders".to_string(),
            },
            CommandHelp {
                name: "remind cancel".to_string(),
                usage: "!remind cancel <id>".to_string(),
                description: "Cancel a pending reminder".to_string(),
            },
        ]
    }

    async fn handle_command(&self, ctx: &CommandContext, db: &Database) -> Option<String> {
        handler::handle_reminder_command(
            &db.pool,
            &ctx.room_id,
            &ctx.sender_mxid,
            &ctx.sender_name,
            &ctx.full_body,
        )
        .await
    }

    async fn spawn_background_workers(&self, ctx: &ModuleContext) {
        scheduler::start_reminder_scheduler(ctx.clone());
    }
}
