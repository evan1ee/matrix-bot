//! Per-user display preferences: timezone and default currency.
//!
//! Unusually for a module, its data is read by nearly every other one: the todo
//! and reminder handlers format timestamps against the sender's zone, the
//! finance handler defaults an amount's currency to it, and `api/settings.rs`
//! exposes the same row to the web UI. A preference therefore reads the same
//! whichever front door produced it.

use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod models;
pub mod repository;

pub use models::{parse_timezone, UserSettings, DEFAULT_CURRENCY, DEFAULT_TIMEZONE};
pub use repository::SettingsRepository;

pub struct SettingsModule;

impl SettingsModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for SettingsModule {
    fn name(&self) -> &str {
        "Settings"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "settings".to_string(),
                usage: "!settings".to_string(),
                description: "Show your timezone and default currency".to_string(),
            },
            CommandHelp {
                name: "settings timezone".to_string(),
                usage: "!settings timezone <IANA zone>".to_string(),
                description: "Set the timezone dates are shown in \
                              (e.g. !settings timezone Asia/Shanghai)"
                    .to_string(),
            },
            CommandHelp {
                name: "settings currency".to_string(),
                usage: "!settings currency <CODE>".to_string(),
                description: "Set the currency assumed when an amount names none \
                              (e.g. !settings currency CNY)"
                    .to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_settings_command(&db.pool, &ctx.sender_mxid, &ctx.full_body, broadcaster)
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
            command: body
                .split_whitespace()
                .next()
                .unwrap_or("")
                .trim_start_matches('!')
                .to_string(),
            raw_args: String::new(),
            full_body: body.to_string(),
        }
    }

    async fn db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
    }

    #[test]
    fn test_advertises_its_commands() {
        let module = SettingsModule::new();
        assert_eq!(module.name(), "Settings");

        let commands = module.commands();
        assert!(commands.iter().any(|c| c.name == "settings timezone"));
        assert!(commands.iter().all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    /// Dispatch returns the first match, so a module that over-claims silently
    /// swallows another's traffic.
    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = SettingsModule::new();
        let db = db().await;

        assert!(module.handle_command(&ctx("!settings"), &db, None).await.is_some());
        assert!(module
            .handle_command(&ctx("!settings tz Asia/Tokyo"), &db, None)
            .await
            .is_some());

        assert!(module.handle_command(&ctx("!todo list"), &db, None).await.is_none());
        assert!(module.handle_command(&ctx("!expense list"), &db, None).await.is_none());
        assert!(module.handle_command(&ctx("hello there"), &db, None).await.is_none());
    }

    /// The setting is only worth having if the other modules observe it.
    #[tokio::test]
    async fn test_a_change_here_is_visible_to_the_todo_module() {
        use crate::modules::todo::TodoModule;

        let db = db().await;
        let settings = SettingsModule::new();
        let todo = TodoModule::new();

        todo.handle_command(&ctx("!todo add ship it"), &db, None).await.unwrap();
        todo.handle_command(&ctx("!todo due 1 2d"), &db, None).await.unwrap();

        settings
            .handle_command(&ctx("!settings timezone Asia/Tokyo"), &db, None)
            .await
            .unwrap();

        let listed = todo.handle_command(&ctx("!todo list"), &db, None).await.unwrap();
        assert!(listed.contains("JST"), "the todo list must follow the setting: {listed}");
    }
}
