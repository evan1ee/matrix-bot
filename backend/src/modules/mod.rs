use async_trait::async_trait;
use matrix_sdk::Client;
use std::sync::Arc;
use tracing::info;

use crate::db::Database;
use crate::events::EventBroadcaster;

pub mod core;
pub mod finance;
pub mod portfolio;
pub mod reminder;
pub mod settings;
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
    ///
    /// `broadcaster` is `None` only where no event stream exists (tests). A module
    /// that mutates state must send its `SystemEvent` through it, exactly as the
    /// REST handlers do — otherwise a change made over Matrix leaves every open
    /// web client showing stale data.
    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String>;

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
    pub async fn dispatch(
        &self,
        ctx: &CommandContext,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        for module in &self.modules {
            if let Some(reply) = module.handle_command(ctx, db, broadcaster).await {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Dispatch returns the *first* module that claims a message, so a command
    /// can be perfectly correct in isolation and still be swallowed by a module
    /// registered ahead of it. This walks the real registration order from
    /// `bot/mod.rs` and asserts each entry point actually comes back.
    #[tokio::test]
    async fn test_every_registered_command_survives_the_real_dispatch_order() {
        use crate::modules::core::CoreModule;
        use crate::modules::finance::FinanceModule;
        use crate::modules::portfolio::PortfolioModule;
        use crate::modules::reminder::ReminderModule;
        use crate::modules::settings::SettingsModule;
        use crate::modules::todo::TodoModule;

        let db = Database::connect("sqlite::memory:", 1).await.unwrap();

        let registry_ref = Arc::new(tokio::sync::RwLock::new(None));
        let mut registry = ModuleRegistry::new();
        registry.register(CoreModule::new(registry_ref.clone()));
        registry.register(TodoModule::new());
        registry.register(ReminderModule::new());
        registry.register(FinanceModule::new());
        registry.register(PortfolioModule::new());
        registry.register(SettingsModule::new());

        for body in [
            "!help",
            "!todo list",
            "!remind list",
            "!expense list",
            "!portfolio",
            "!portfolio buy AAPL 10 @ 150.00 USD",
            "!settings",
            "!settings timezone Asia/Tokyo",
            "!settings currency CNY",
        ] {
            let ctx = CommandContext {
                room_id: "!room:a".to_string(),
                sender_mxid: "@alice:a".to_string(),
                sender_name: "alice".to_string(),
                command: body.split_whitespace().next().unwrap().trim_start_matches('!').to_string(),
                raw_args: String::new(),
                full_body: body.to_string(),
            };
            assert!(
                registry.dispatch(&ctx, &db, None).await.is_some(),
                "`{body}` reached no module"
            );
        }
    }
use crate::events::EventBroadcaster;

    /// A module that claims only the command it was told to claim.
    struct StubModule {
        label: &'static str,
        claims: &'static str,
    }

    #[async_trait]
    impl BotModule for StubModule {
        fn name(&self) -> &str {
            self.label
        }

        fn commands(&self) -> Vec<CommandHelp> {
            vec![CommandHelp {
                name: self.claims.to_string(),
                usage: format!("!{}", self.claims),
                description: format!("stub for {}", self.claims),
            }]
        }

        async fn handle_command(
            &self,
            ctx: &CommandContext,
            _db: &Database,
            _broadcaster: Option<&EventBroadcaster>,
        ) -> Option<String> {
            (ctx.command == self.claims).then(|| format!("handled by {}", self.label))
        }
    }

    fn ctx_for(command: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@alice:a".to_string(),
            sender_name: "alice".to_string(),
            command: command.to_string(),
            raw_args: String::new(),
            full_body: format!("!{}", command),
        }
    }

    async fn memory_db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
    }

    /// Dispatch returns the *first* module that claims the command. Registration
    /// order is therefore a real contract, not an implementation detail.
    #[tokio::test]
    async fn test_dispatch_returns_the_first_claiming_module() {
        let db = memory_db().await;
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule { label: "first", claims: "shared" });
        registry.register(StubModule { label: "second", claims: "shared" });

        let reply = registry.dispatch(&ctx_for("shared"), &db, None).await;
        assert_eq!(reply.as_deref(), Some("handled by first"));
    }

    /// An unclaimed command must fall through as `None` so the bot stays silent
    /// rather than answering with something arbitrary.
    #[tokio::test]
    async fn test_unclaimed_command_falls_through() {
        let db = memory_db().await;
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule { label: "todo", claims: "todo" });

        assert!(registry.dispatch(&ctx_for("nonsense"), &db, None).await.is_none());
    }

    #[tokio::test]
    async fn test_dispatch_reaches_a_later_module() {
        let db = memory_db().await;
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule { label: "todo", claims: "todo" });
        registry.register(StubModule { label: "finance", claims: "expense" });

        let reply = registry.dispatch(&ctx_for("expense"), &db, None).await;
        assert_eq!(reply.as_deref(), Some("handled by finance"));
    }

    /// Help generation must cover every registered module, or a command becomes
    /// undiscoverable the moment its module is added.
    #[test]
    fn test_help_covers_every_registered_module() {
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule { label: "todo", claims: "todo" });
        registry.register(StubModule { label: "finance", claims: "expense" });

        let help = registry.get_all_help();
        assert_eq!(help.len(), 2);
        assert_eq!(help[0].0, "todo");
        assert_eq!(help[1].0, "finance");
        assert!(help.iter().all(|(_, cmds)| !cmds.is_empty()));
    }
}
