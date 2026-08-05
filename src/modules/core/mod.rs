use async_trait::async_trait;
use std::sync::Arc;

use crate::db::Database;
use crate::modules::{BotModule, CommandContext, CommandHelp, ModuleRegistry};
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

#[async_trait]
impl BotModule for CoreModule {
    fn name(&self) -> &str {
        "Core"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "hello".to_string(),
                usage: "!hello [name]".to_string(),
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

    async fn handle_command(&self, ctx: &CommandContext, _db: &Database) -> Option<String> {
        let cmd_lower = ctx.command.to_lowercase();
        match cmd_lower.as_str() {
            "hello" | "hi" => {
                if ctx.raw_args.is_empty() {
                    Some(format!(
                        "👋 Hello, {}! Greetings from <b>matrix-hello-bot</b> written in Rust! 🦀",
                        html_escape(&ctx.sender_name)
                    ))
                } else {
                    Some(format!(
                        "👋 Hello, <b>{}</b>! Greetings from Rust! 🦀",
                        html_escape(&ctx.raw_args)
                    ))
                }
            }
            "ping" => Some("🏓 <b>Pong!</b> Matrix Hello Bot is active and running!".to_string()),
            "help" => {
                let guard = self.registry_ref.read().await;
                if let Some(registry) = guard.as_ref() {
                    let helps = registry.get_all_help();
                    Some(help::generate_help_html(&helps))
                } else {
                    Some("🤖 <b>Matrix Hello Bot</b> (Core commands: !hello, !ping, !help)".to_string())
                }
            }
            _ => None,
        }
    }
}
