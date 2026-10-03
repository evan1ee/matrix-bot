use async_trait::async_trait;

use crate::interface::channels::message::Reply;
use std::sync::Arc;

use crate::db::Database;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp, ModuleRegistry};
use crate::utils::html_escape;

pub mod help;

pub struct CoreModule {
    registry_ref: Arc<tokio::sync::RwLock<Option<Arc<ModuleRegistry>>>>,
}

impl CoreModule {
    pub fn new(registry_ref: Arc<tokio::sync::RwLock<Option<Arc<ModuleRegistry>>>>) -> Self {
        Self { registry_ref }
    }
}

impl CoreModule {
    /// The command's HTML answer; `handle_command` wraps it in a [`Reply`].
    async fn respond(&self, ctx: &CommandContext) -> Option<String> {
        let cmd_lower = ctx.command.to_lowercase();
        match cmd_lower.as_str() {
            "hello" | "hi" => {
                if ctx.raw_args.is_empty() {
                    Some(format!(
                        "👋 Hello, {}! Greetings from <b>matrix-bot</b> written in Rust! 🦀",
                        html_escape(ctx.sender_name())
                    ))
                } else {
                    Some(format!(
                        "👋 Hello, <b>{}</b>! Greetings from Rust! 🦀",
                        html_escape(&ctx.raw_args)
                    ))
                }
            }
            "ping" => Some("🏓 <b>Pong!</b> Matrix Bot is active and running!".to_string()),
            "help" => {
                let guard = self.registry_ref.read().await;
                if let Some(registry) = guard.as_ref() {
                    let helps = registry.get_all_help();
                    Some(help::generate_help_html(&helps))
                } else {
                    Some("🤖 <b>Matrix Bot</b> (Core commands: !hello, !ping, !help)".to_string())
                }
            }
            _ => None,
        }
    }
}

#[async_trait]
impl BotModule for CoreModule {
    fn name(&self) -> &str {
        "Core"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "hello".to_string(),
                usage: "!hello / !hi [name]".to_string(),
                description: "Get a friendly greeting".to_string(),
            },
            CommandHelp {
                name: "ping".to_string(),
                usage: "!ping".to_string(),
                description: "Check bot availability and latency".to_string(),
            },
            CommandHelp {
                name: "help".to_string(),
                usage: "!help".to_string(),
                description: "View command list and usage help".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        _services: &crate::application::AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        self.respond(ctx).await.map(Reply::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_core_commands() {
        let registry_ref = Arc::new(tokio::sync::RwLock::new(None));
        let core = CoreModule::new(registry_ref);

        let ctx = CommandContext {
            conversation: crate::domain::channels::ConversationAddress::Matrix(
                "!room:1".to_string(),
            ),
            owner_id: crate::domain::accounts::AccountId::from_seed("@user:1"),
            sender: crate::interface::chat::test_support::sender("@tester:x", "Alice"),
            command: "hello".to_string(),
            raw_args: "".to_string(),
            full_body: "!hello".to_string(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            progress: None,
            outbound: None,
        };

        let dummy_db = Database {
            pool: crate::db::pool::test_pool().await,
        };

        let res = core
            .handle_command(
                &ctx,
                &crate::application::AppServices::new(&dummy_db.pool),
                &dummy_db,
            )
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(res.contains("Hello, Alice!"));

        let ctx_ping = CommandContext {
            command: "ping".to_string(),
            ..ctx
        };
        let res_ping = core
            .handle_command(
                &ctx_ping,
                &crate::application::AppServices::new(&dummy_db.pool),
                &dummy_db,
            )
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(res_ping.contains("Pong!"));
    }
}
