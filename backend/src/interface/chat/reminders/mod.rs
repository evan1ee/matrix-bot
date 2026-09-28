//! Reminder Matrix chat module.

use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp, ModuleContext};

pub mod handler;
pub mod scheduler;

pub struct ReminderModule;

impl Default for ReminderModule {
    fn default() -> Self {
        Self::new()
    }
}

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
                description: "Set a one-time or recurring timer (e.g., !remind 10s check oven)"
                    .to_string(),
            },
            CommandHelp {
                name: "remind list".to_string(),
                usage: "!remind list / !reminders".to_string(),
                description: "List your active pending reminders".to_string(),
            },
            CommandHelp {
                name: "remind cancel".to_string(),
                usage: "!remind cancel <id> / !remind rm <id>".to_string(),
                description: "Cancel a pending reminder".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        _db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_reminder_command(&services.reminders, &services.settings, ctx).await
    }

    async fn spawn_background_workers(
        &self,
        ctx: &ModuleContext,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        vec![scheduler::start_reminder_scheduler(ctx.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::chat::CommandContext;

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@tester:x".to_string(),
            owner_id: crate::domain::accounts::AccountId::new("@alice:a"),
            sender_name: "alice".to_string(),
            command: body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('!')
                .to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        Database { pool }
    }

    #[test]
    fn test_advertises_its_commands() {
        let module = ReminderModule::new();
        assert_eq!(module.name(), "Reminder");
        let commands = module.commands();
        assert!(!commands.is_empty());
        assert!(commands
            .iter()
            .all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = ReminderModule::new();
        let db = db().await;

        assert!(module
            .handle_command(
                &ctx("!remind list"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None
            )
            .await
            .is_some());
        assert!(module
            .handle_command(
                &ctx("!todo list"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None
            )
            .await
            .is_none());
        assert!(module
            .handle_command(
                &ctx("!expense list"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None
            )
            .await
            .is_none());
    }
}
