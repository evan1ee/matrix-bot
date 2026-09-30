//! Chat front door, one file per feature: each feature's Matrix side — its
//! `BotModule` command handler, and any Matrix-specific lookups it needs —
//! calling that feature's application service. Holds no SQL
//! (`tests/architecture.rs`).

pub mod account;
pub mod bookmarks;
pub mod briefing;
pub mod core;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod journal;
pub mod life;
pub mod llm;
pub mod mcp;
pub mod ocr;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod todos;
pub mod tts;
pub mod voice;
pub mod webhooks;

use std::sync::Arc;

use async_trait::async_trait;
use matrix_sdk::Client;
use tokio::task::JoinHandle;
use tracing::{error, info};

use crate::application::AppError;
use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::utils::html::html_escape;

/// A message the parser recognised as a command, before its sender has been
/// resolved to an account.
///
/// Kept separate from [`CommandContext`] so the resolver query runs *only* for
/// a message that is actually a command. Folding the two together would mean
/// either looking up an account for every line of unrelated room chatter, or
/// carrying a `CommandContext` whose `owner_id` is empty until someone
/// remembers to fill it — and an ownership key that is briefly the empty string
/// is a key that will eventually be written to a row.
#[derive(Clone, Debug)]
pub struct ParsedCommand {
    /// The Matrix room ID where the command was sent.
    pub room_id: String,
    /// The display name of the sender.
    pub sender_name: String,
    /// The parsed command keyword (e.g. `todo`, `remind`).
    pub command: String,
    /// The raw argument string following the command keyword.
    pub raw_args: String,
    /// The full original message body.
    pub full_body: String,
    /// Whether the bot was explicitly mentioned (@bot or pill).
    pub is_mentioned: bool,
    /// Optional quoted message context if the message is a reply or quote.
    pub quoted: Option<crate::interface::channels::matrix::quote::QuotedMessage>,
}

impl ParsedCommand {
    /// Binds this command to the account that will own anything it creates.
    pub fn for_owner(
        self,
        owner_id: impl Into<crate::domain::accounts::AccountId>,
        sender_mxid: impl Into<String>,
        is_direct: bool,
    ) -> CommandContext {
        CommandContext {
            room_id: self.room_id,
            owner_id: owner_id.into(),
            sender_mxid: sender_mxid.into(),
            sender_name: self.sender_name,
            command: self.command,
            raw_args: self.raw_args,
            full_body: self.full_body,
            is_direct,
            is_mentioned: self.is_mentioned,
            quoted: self.quoted,
            stage_reporter: None,
        }
    }
}

/// Context passed to a module's `handle_command()` with parsed message details.
#[derive(Clone, Debug)]
pub struct CommandContext {
    /// The Matrix room ID where the command was sent.
    pub room_id: String,
    /// The account that owns anything this command creates.
    ///
    /// An `accounts.id`, resolved from the sender's MXID through `matrix_links`
    /// — never the MXID itself. Two MXIDs linked to the same account produce
    /// the same value here, which is what makes a user's data follow them
    /// across Matrix identities and the web UI alike.
    pub owner_id: crate::domain::accounts::AccountId,
    /// The Matrix identity that sent this, for provenance and for the two
    /// commands that act on a Matrix account rather than on data.
    ///
    /// Never an ownership key: two MXIDs linked to one account are the same
    /// owner, and `owner_id` is the only thing that says so.
    pub sender_mxid: String,
    /// The display name of the sender. Display only — it records who typed the
    /// command in a room, and must never appear in an ownership predicate.
    pub sender_name: String,
    /// The parsed command keyword (e.g. `todo`, `remind`).
    pub command: String,
    /// The raw argument string following the command keyword.
    pub raw_args: String,
    /// The full original message body.
    pub full_body: String,
    /// Whether this arrived in a one-to-one room with the bot.
    ///
    /// `!link` and `!recover` refuse anything else: both put a secret in the
    /// room, and a shared room hands it to everyone present.
    pub is_direct: bool,
    /// Whether the bot was explicitly mentioned in the room.
    pub is_mentioned: bool,
    /// Optional quoted message context if the message is a reply or quote.
    pub quoted: Option<crate::interface::channels::matrix::quote::QuotedMessage>,
    /// Optional in-place stage reporter for tracking multi-phase progress in Matrix.
    pub stage_reporter: Option<crate::interface::channels::matrix::stage::StageReporter>,
}

/// Shared runtime context available to modules for background workers.
#[derive(Clone)]
pub struct ModuleContext {
    /// The Matrix SDK client for sending messages.
    pub client: Client,
    /// The shared database connection pool.
    pub db: Database,
    /// Application services, built once in `app.rs`.
    pub services: crate::application::AppServices,
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
    ///
    /// `services` is the application layer built once in `app.rs`; a moved
    /// feature's command handler calls its service through it. Modules not
    /// yet moved ignore it.
    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String>;

    /// Spawns any background tasks (e.g. scheduler loops).
    ///
    /// Returns their handles so the caller can stop them. A worker holds the
    /// `ModuleContext` it was given — including that generation's Matrix client
    /// — so it must not outlive the sync session that produced it; see
    /// [`BackgroundWorkers`].
    async fn spawn_background_workers(&self, _ctx: &ModuleContext) -> Vec<JoinHandle<()>> {
        Vec::new()
    }
}

/// Central registry holding all registered `BotModule` instances.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: Vec<Arc<dyn BotModule>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
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
        services: &crate::application::AppServices,
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        for module in &self.modules {
            if let Some(reply) = module.handle_command(ctx, services, db, broadcaster).await {
                return Some(reply);
            }
        }
        None
    }

    /// Spawns background workers for all registered modules.
    ///
    /// The returned guard owns them: drop it and they stop.
    pub async fn spawn_all_background_workers(&self, ctx: &ModuleContext) -> BackgroundWorkers {
        let mut handles = Vec::new();
        for module in &self.modules {
            handles.extend(module.spawn_background_workers(ctx).await);
        }
        BackgroundWorkers { handles }
    }

    /// Returns help text for all modules, grouped by module name.
    pub fn get_all_help(&self) -> Vec<(String, Vec<CommandHelp>)> {
        self.modules
            .iter()
            .map(|m| (m.name().to_string(), m.commands()))
            .collect()
    }
}

/// Owns a generation of background workers and stops them when dropped.
///
/// The Matrix sync loop reconnects, and each reconnect builds a fresh client and
/// calls `run()` again. Spawning detached workers there leaked one scheduler per
/// reconnect: every surviving copy kept polling the same `reminders` table, so a
/// single due reminder was delivered once per accumulated worker — the bug where
/// one reminder arrived three times, then four after another reconnect.
///
/// Tying the workers to a guard makes the lifetime explicit: they belong to the
/// sync session that spawned them, and they hold that session's client, so they
/// must not outlive it. Aborting mid-delivery is safe — the row stays `pending`
/// and the next generation redelivers it, which is the same at-least-once
/// behaviour a crash already implied.
pub struct BackgroundWorkers {
    handles: Vec<JoinHandle<()>>,
}

impl BackgroundWorkers {
    /// How many workers this guard owns.
    pub fn len(&self) -> usize {
        self.handles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }
}

impl Drop for BackgroundWorkers {
    fn drop(&mut self) {
        for handle in &self.handles {
            handle.abort();
        }
    }
}

/// What a failed use case tells the user in chat: the chat counterpart of
/// `interface/http/app_error.rs`, and the only place a command handler
/// turns an `AppError` into a reply.
///
/// Returns the HTML reply a `BotModule` hands back. The four user-facing
/// variants show their (escaped) message; a database or external failure is
/// logged here and answered with a fixed apology, so a SQL or provider error
/// never reaches a room.
pub fn error_reply(err: AppError) -> String {
    let message = match err {
        AppError::Validation(msg)
        | AppError::NotFound(msg)
        | AppError::Forbidden(msg)
        | AppError::Conflict(msg) => msg,
        AppError::Database(err) => {
            error!(error = %err, "Chat command failed on the database");
            return INTERNAL_REPLY.to_owned();
        }
        AppError::External { service, source } => {
            error!(service, error = %format!("{source:#}"), "Chat command failed on a dependency");
            return INTERNAL_REPLY.to_owned();
        }
    };
    format!("⚠️ {}", html_escape(&message))
}

const INTERNAL_REPLY: &str = "⚠️ Something went wrong on our side. Please try again later.";

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn user_facing_errors_show_their_escaped_message() {
        let reply = error_reply(AppError::Validation("<b>too long</b>".into()));
        assert_eq!(reply, "⚠️ &lt;b&gt;too long&lt;/b&gt;");
        assert_eq!(
            error_reply(AppError::NotFound("No such todo".into())),
            "⚠️ No such todo"
        );
    }

    #[test]
    fn diagnostic_errors_never_reach_the_room() {
        let db = error_reply(AppError::Database(sqlx::Error::Protocol(
            "relation \"account_rooms\" does not exist".into(),
        )));
        let ext = error_reply(AppError::External {
            service: "ocr",
            source: anyhow::anyhow!("10.0.0.5 refused"),
        });
        assert_eq!(db, INTERNAL_REPLY);
        assert_eq!(ext, INTERNAL_REPLY);
    }

    /// Dropping the guard must actually stop the workers it owns.
    #[tokio::test]
    async fn test_dropping_the_guard_stops_its_workers() {
        static TICKS: AtomicUsize = AtomicUsize::new(0);

        let spawn_ticker = || {
            tokio::spawn(async {
                loop {
                    TICKS.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
        };

        let workers = BackgroundWorkers {
            handles: vec![spawn_ticker(), spawn_ticker()],
        };
        assert_eq!(workers.len(), 2);

        tokio::time::sleep(std::time::Duration::from_millis(40)).await;
        assert!(
            TICKS.load(Ordering::SeqCst) > 0,
            "the workers should be running before the guard is dropped"
        );

        drop(workers);
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        let after_abort = TICKS.load(Ordering::SeqCst);

        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert_eq!(
            TICKS.load(Ordering::SeqCst),
            after_abort,
            "a dropped guard must leave no worker still ticking"
        );
    }

    /// Dispatch returns the *first* module that claims a message, so a command
    /// can be perfectly correct in isolation and still be swallowed by a module
    /// registered ahead of it. This walks the real registration order from
    /// `bot/mod.rs` and asserts each entry point actually comes back.
    #[tokio::test]
    async fn test_every_registered_command_survives_the_real_dispatch_order() {
        use crate::interface::chat::core::CoreModule;
        use crate::interface::chat::finance::FinanceModule;
        use crate::interface::chat::portfolio::PortfolioModule;
        use crate::interface::chat::reminders::ReminderModule;
        use crate::interface::chat::settings::SettingsModule;
        use crate::interface::chat::todos::TodoModule;
        use crate::interface::chat::tts::TtsModule;

        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let db = Database { pool };

        let registry_ref = Arc::new(tokio::sync::RwLock::new(None));
        let mut registry = ModuleRegistry::new();
        registry.register(CoreModule::new(registry_ref.clone()));
        registry.register(TodoModule::new());
        registry.register(ReminderModule::new());
        registry.register(FinanceModule::new());
        registry.register(PortfolioModule::new());
        registry.register(SettingsModule::new());
        registry.register(TtsModule::new(
            Arc::new(tokio::sync::RwLock::new(None)),
            crate::infrastructure::inference::tts::TtsClient::new(
                None,
                None,
                "kokoro".to_string(),
                crate::infrastructure::inference::tts::TtsDialect::OpenAi,
            ),
            "0".to_string(),
        ));

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
            "!say hello",
        ] {
            let ctx = CommandContext {
                room_id: "!room:a".to_string(),
                sender_mxid: "@tester:x".to_string(),
                owner_id: crate::domain::accounts::AccountId::from_seed("@alice:a"),
                sender_name: "alice".to_string(),
                command: body
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .trim_start_matches('!')
                    .to_string(),
                raw_args: String::new(),
                full_body: body.to_string(),
                is_direct: true,
                is_mentioned: false,
                quoted: None,
                stage_reporter: None,
            };
            assert!(
                registry
                    .dispatch(
                        &ctx,
                        &crate::application::AppServices::new(&db.pool),
                        &db,
                        None
                    )
                    .await
                    .is_some(),
                "`{body}` reached no module"
            );
        }
    }

    #[tokio::test]
    async fn test_tts_must_be_registered_before_a_direct_message_catch_all() {
        use crate::interface::chat::tts::TtsModule;

        struct DirectMessageCatchAll;

        #[async_trait]
        impl BotModule for DirectMessageCatchAll {
            fn name(&self) -> &str {
                "catch-all"
            }

            fn commands(&self) -> Vec<CommandHelp> {
                Vec::new()
            }

            async fn handle_command(
                &self,
                ctx: &CommandContext,
                _services: &crate::application::AppServices,
                _db: &Database,
                _broadcaster: Option<&EventBroadcaster>,
            ) -> Option<String> {
                ctx.is_direct
                    .then(|| "answered by the catch-all".to_string())
            }
        }

        let db = memory_db().await;
        let tts_module = || {
            TtsModule::new(
                Arc::new(tokio::sync::RwLock::new(None)),
                crate::infrastructure::inference::tts::TtsClient::new(
                    None,
                    None,
                    "kokoro".to_string(),
                    crate::infrastructure::inference::tts::TtsDialect::OpenAi,
                ),
                "0".to_string(),
            )
        };

        let mut correct = ModuleRegistry::new();
        correct.register(tts_module());
        correct.register(DirectMessageCatchAll);
        let reply = correct
            .dispatch(
                &ctx_for("say"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None,
            )
            .await
            .expect("TTS claims `say`");
        assert!(
            !reply.contains("catch-all"),
            "TTS must answer `!say` before a direct-message catch-all gets a chance"
        );

        let mut swapped = ModuleRegistry::new();
        swapped.register(DirectMessageCatchAll);
        swapped.register(tts_module());
        let reply = swapped
            .dispatch(
                &ctx_for("say"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None,
            )
            .await
            .expect("the catch-all claims every direct message");
        assert!(
            reply.contains("catch-all"),
            "sanity check: misordering really does let the catch-all swallow `!say`"
        );
    }

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
            _services: &crate::application::AppServices,
            _db: &Database,
            _broadcaster: Option<&EventBroadcaster>,
        ) -> Option<String> {
            (ctx.command == self.claims).then(|| format!("handled by {}", self.label))
        }
    }

    fn ctx_for(command: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@tester:x".to_string(),
            owner_id: crate::domain::accounts::AccountId::from_seed("@alice:a"),
            sender_name: "alice".to_string(),
            command: command.to_string(),
            raw_args: String::new(),
            full_body: format!("!{}", command),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn memory_db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        Database { pool }
    }

    #[tokio::test]
    async fn test_dispatch_returns_the_first_claiming_module() {
        let db = memory_db().await;
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule {
            label: "first",
            claims: "shared",
        });
        registry.register(StubModule {
            label: "second",
            claims: "shared",
        });

        let reply = registry
            .dispatch(
                &ctx_for("shared"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None,
            )
            .await;
        assert_eq!(reply.as_deref(), Some("handled by first"));
    }

    #[tokio::test]
    async fn test_unclaimed_command_falls_through() {
        let db = memory_db().await;
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule {
            label: "todo",
            claims: "todo",
        });

        assert!(registry
            .dispatch(
                &ctx_for("nonsense"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None
            )
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_dispatch_reaches_a_later_module() {
        let db = memory_db().await;
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule {
            label: "todo",
            claims: "todo",
        });
        registry.register(StubModule {
            label: "finance",
            claims: "expense",
        });

        let reply = registry
            .dispatch(
                &ctx_for("expense"),
                &crate::application::AppServices::new(&db.pool),
                &db,
                None,
            )
            .await;
        assert_eq!(reply.as_deref(), Some("handled by finance"));
    }

    #[test]
    fn test_help_covers_every_registered_module() {
        let mut registry = ModuleRegistry::new();
        registry.register(StubModule {
            label: "todo",
            claims: "todo",
        });
        registry.register(StubModule {
            label: "finance",
            claims: "expense",
        });

        let help = registry.get_all_help();
        assert_eq!(help.len(), 2);
        assert_eq!(help[0].0, "todo");
        assert_eq!(help[1].0, "finance");
        assert!(help.iter().all(|(_, cmds)| !cmds.is_empty()));
    }
}
