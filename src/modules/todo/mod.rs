use async_trait::async_trait;

use crate::db::Database;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod models;
pub mod repository;

pub struct TodoModule;

impl TodoModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for TodoModule {
    fn name(&self) -> &str {
        "To-Do"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "todo add".to_string(),
                usage: "!todo add <task>".to_string(),
                description: "Add a new task to your list".to_string(),
            },
            CommandHelp {
                name: "todo list".to_string(),
                usage: "!todo [list|my|room]".to_string(),
                description: "View your personal or room-wide tasks".to_string(),
            },
            CommandHelp {
                name: "todo done".to_string(),
                usage: "!todo done <id>".to_string(),
                description: "Mark a task #id as completed".to_string(),
            },
            CommandHelp {
                name: "todo priority".to_string(),
                usage: "!todo priority <id> <low|medium|high>".to_string(),
                description: "Change task priority".to_string(),
            },
            CommandHelp {
                name: "todo edit".to_string(),
                usage: "!todo edit <id> <new text>".to_string(),
                description: "Edit task description".to_string(),
            },
            CommandHelp {
                name: "todo clear".to_string(),
                usage: "!todo clear [room]".to_string(),
                description: "Remove completed tasks".to_string(),
            },
        ]
    }

    async fn handle_command(&self, ctx: &CommandContext, db: &Database) -> Option<String> {
        handler::handle_todo_command(
            &db.pool,
            &ctx.room_id,
            &ctx.sender_mxid,
            &ctx.sender_name,
            &ctx.full_body,
        )
        .await
    }
}
