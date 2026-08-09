use async_trait::async_trait;

use crate::db::Database;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod due_reminder;
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
                usage: "!todo add [[high|low]] <task>".to_string(),
                description: "Add a new task (supports inline priority like [high])".to_string(),
            },
            CommandHelp {
                name: "todo list".to_string(),
                usage: "!todo [list|room] [page]".to_string(),
                description: "View personal or room tasks with pagination".to_string(),
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
                name: "todo due".to_string(),
                usage: "!todo due <id> <time> [remind <lead>]".to_string(),
                description: "Set a task due date, optionally with a reminder beforehand \
                              (e.g. !todo due 1 2d remind 1h). Use 'none' to clear it."
                    .to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::CommandContext;

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@alice:a".to_string(),
            sender_name: "alice".to_string(),
            command: body.split_whitespace().next().unwrap_or("").trim_start_matches('!').to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
        }
    }

    async fn db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
    }

    #[test]
    fn test_advertises_its_commands() {
        let module = TodoModule::new();
        assert_eq!(module.name(), "To-Do");

        let commands = module.commands();
        assert!(commands.iter().any(|c| c.name == "todo due"));
        // The due command advertises the reminder suffix, or nobody discovers it.
        let due = commands.iter().find(|c| c.name == "todo due").unwrap();
        assert!(due.usage.contains("remind"), "usage should mention the reminder: {}", due.usage);
        assert!(commands.iter().all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = TodoModule::new();
        let db = db().await;

        assert!(module.handle_command(&ctx("!todo list"), &db).await.is_some());
        assert!(module.handle_command(&ctx("!expense add 1 EUR x"), &db).await.is_none());
        assert!(module.handle_command(&ctx("!remind me later"), &db).await.is_none());
    }

    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let module = TodoModule::new();
        let db = db().await;

        module.handle_command(&ctx("!todo add <img src=x onerror=1>"), &db).await.unwrap();
        let listed = module.handle_command(&ctx("!todo list"), &db).await.unwrap();

        assert!(!listed.contains("<img"), "raw markup must not survive: {listed}");
        assert!(listed.contains("&lt;img"));
    }
}
