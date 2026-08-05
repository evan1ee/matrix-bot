use async_trait::async_trait;
use matrix_sdk::Client;
use std::sync::Arc;
use tracing::info;

use crate::db::Database;

pub mod core;
pub mod reminder;
pub mod todo;

/// Context passed to a module's `handle_command()` with parsed message details.
#[derive(Clone, Debug)]
pub struct CommandContext {
    /// The Matrix room ID where the command was sent.
    pub room_id: String,
    /// The full Matrix user ID of the sender (e.g. `@alice:matrix.org`).
    pub sender_mxid: String,
    /// The display name of the sender.
    pub sender_name: String,
    /// The parsed command keyword (e.g. `todo`, `remind`).
    pub command: String,
    /// The raw argument string following the command keyword.
    pub raw_args: String,
    /// The full original message body.
    pub full_body: String,
}

/// Shared runtime context available to modules for background workers.
#[derive(Clone)]
pub struct ModuleContext {
    /// The Matrix SDK client for sending messages.
    pub client: Client,
    /// The shared database connection pool.
    pub db: Database,
    /// Optional event broadcaster for real-time SSE streaming.
    pub broadcaster: Option<crate::events::EventBroadcaster>,
}

/// Describes a single command's usage and purpose for help generation.
#[derive(Clone, Debug)]
pub struct CommandHelp {
    #[allow(dead_code)]
    pub name: String,
    pub usage: String,
    pub description: String,
}

/// Trait that all feature modules must implement to be registered in the bot.
///
/// Each module handles a set of commands and can optionally spawn background workers
/// (e.g. the reminder scheduler) and run database migrations for its own tables.
#[async_trait]
pub trait BotModule: Send + Sync {
    /// Returns the module's display name (e.g. "Core", "To-Do", "Reminder").
    fn name(&self) -> &str;

    /// Returns a list of commands this module handles, for help text generation.
    fn commands(&self) -> Vec<CommandHelp>;

    /// Attempts to handle a command. Returns `Some(html_reply)` if handled, `None` to pass.
    async fn handle_command(&self, ctx: &CommandContext, db: &Database) -> Option<String>;

    /// Spawns any background tasks (e.g. scheduler loops). Called once at startup.
    async fn spawn_background_workers(&self, _ctx: &ModuleContext) {}
}

/// Central registry holding all registered `BotModule` instances.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn BotModule>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self { modules: Vec::new() }
    }

    /// Registers a new module. Called during bot initialization.
    pub fn register<M: BotModule + 'static>(&mut self, module: M) {
        info!("Registering bot module: {}", module.name());
        self.modules.push(Arc::new(module));
    }

    /// Dispatches a command to all modules in registration order, returning the first match.
    pub async fn dispatch(&self, ctx: &CommandContext, db: &Database) -> Option<String> {
        for module in &self.modules {
            if let Some(reply) = module.handle_command(ctx, db).await {
                return Some(reply);
            }
        }
        None
    }

    /// Spawns background workers for all registered modules.
    pub async fn spawn_all_background_workers(&self, ctx: &ModuleContext) {
        for module in &self.modules {
            module.spawn_background_workers(ctx).await;
        }
    }

    /// Returns help text for all modules, grouped by module name.
    pub fn get_all_help(&self) -> Vec<(String, Vec<CommandHelp>)> {
        self.modules
            .iter()
            .map(|m| (m.name().to_string(), m.commands()))
            .collect()
    }
}
