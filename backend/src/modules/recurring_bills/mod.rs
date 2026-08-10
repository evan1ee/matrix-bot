use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod due_reminder;
pub mod handler;
pub mod models;
pub mod repository;

pub struct RecurringBillsModule;

impl RecurringBillsModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RecurringBillsModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for RecurringBillsModule {
    fn name(&self) -> &str {
        "Recurring Bills"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "bill list".to_string(),
                usage: "!bill list".to_string(),
                description: "List all recurring bills, subscriptions, installments, and transfers".to_string(),
            },
            CommandHelp {
                name: "bill add".to_string(),
                usage: "!bill add <name> <amount> <currency> <category> [frequency]".to_string(),
                description: "Add a new recurring bill or subscription".to_string(),
            },
            CommandHelp {
                name: "bill pay".to_string(),
                usage: "!bill pay <id>".to_string(),
                description: "Mark a recurring bill as paid for the current period".to_string(),
            },
            CommandHelp {
                name: "bill remind".to_string(),
                usage: "!bill remind <id> <minutes|off>".to_string(),
                description: "Remind me this many minutes before the bill is due".to_string(),
            },
            CommandHelp {
                name: "bill delete".to_string(),
                usage: "!bill delete <id>".to_string(),
                description: "Delete a recurring bill".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        if ctx.command == "bill" || ctx.command == "recurring" {
            handler::handle_bill_command(ctx, db, broadcaster).await
        } else {
            None
        }
    }
}
