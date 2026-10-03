//! Chat front door, one file per feature: each feature's Matrix side — its
//! `BotModule` command handler, and any Matrix-specific lookups it needs —
//! calling that feature's application service. Holds no SQL
//! (`tests/architecture.rs`).

pub mod account;
pub mod agent;
pub mod bookmarks;
pub mod briefing;
pub mod core;
pub mod finance;
pub mod footprints;
pub mod habits;
pub mod journal;
pub mod life;
pub mod link;
pub mod llm;
pub mod llm_alert;
pub mod mcp;
pub mod ocr;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod settings;
pub mod todos;
pub mod tts;
pub mod voice;
pub mod webhooks;

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use tracing::{error, info};

use crate::application::AppError;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity};
use crate::domain::chat::QuotedMessage;
use crate::interface::channels::message::Reply;
use crate::interface::channels::registry::ChannelRegistry;
use crate::llm::progress::ProgressSink;
use crate::utils::html::html_escape;

/// The name shown for "who typed this": the platform's display name when the
/// adapter supplied one, else the user id's localpart for Matrix (what the
/// Matrix parser has always recorded) or the whole id elsewhere.
fn display_name_of(sender: &PlatformIdentity) -> &str {
    if let Some(name) = sender.display_name() {
        return name;
    }
    let id = sender.user_id().as_str();
    match sender.channel() {
        Channel::Matrix => id.trim_start_matches('@').split(':').next().unwrap_or(id),
        _ => id,
    }
}

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
    /// Where the command was sent.
    pub conversation: ConversationAddress,
    /// Who sent it.
    pub sender: PlatformIdentity,
    /// The parsed command keyword (e.g. `todo`, `remind`).
    pub command: String,
    /// The raw argument string following the command keyword.
    pub raw_args: String,
    /// The full original message body.
    pub full_body: String,
    /// Whether the bot was explicitly mentioned (@bot or pill).
    pub is_mentioned: bool,
    /// Optional quoted message context if the message is a reply or quote.
    pub quoted: Option<QuotedMessage>,
}

impl ParsedCommand {
    /// Display name of the sender; see [`CommandContext::sender_name`].
    pub fn sender_name(&self) -> &str {
        display_name_of(&self.sender)
    }

    /// Binds this command to the account that will own anything it creates.
    pub fn for_owner(self, owner_id: AccountId, is_direct: bool) -> CommandContext {
        CommandContext {
            conversation: self.conversation,
            owner_id,
            sender: self.sender,
            command: self.command,
            raw_args: self.raw_args,
            full_body: self.full_body,
            is_direct,
            is_mentioned: self.is_mentioned,
            quoted: self.quoted,
            progress: None,
            outbound: None,
        }
    }
}

/// Context passed to a module's `handle_command()` with parsed message details.
#[derive(Clone)]
pub struct CommandContext {
    /// Where the command was sent.
    pub conversation: ConversationAddress,
    /// The account that owns anything this command creates.
    ///
    /// An `accounts.id`, resolved from the sender's platform identity through
    /// `platform_identities` — never the platform user id itself. Two
    /// identities linked to the same account produce the same value here,
    /// which is what makes a user's data follow them across platforms and the
    /// web UI alike.
    pub owner_id: AccountId,
    /// The platform identity that sent this, for provenance and for the
    /// commands that act on a platform account rather than on data.
    ///
    /// Never an ownership key: `owner_id` is the only thing that says who owns.
    pub sender: PlatformIdentity,
    /// The parsed command keyword (e.g. `todo`, `remind`).
    pub command: String,
    /// The raw argument string following the command keyword.
    pub raw_args: String,
    /// The full original message body.
    pub full_body: String,
    /// Whether this arrived in a one-to-one conversation with the bot.
    ///
    /// `!recover` refuses anything else: it puts a secret in the
    /// conversation, and a shared one hands it to everyone present.
    pub is_direct: bool,
    /// Whether the bot was explicitly mentioned in the conversation.
    pub is_mentioned: bool,
    /// Optional quoted message context if the message is a reply or quote.
    pub quoted: Option<QuotedMessage>,
    /// Where a multi-phase turn reports its progress, when the adapter shows any.
    pub progress: Option<Arc<dyn ProgressSink>>,
    /// Out-of-band sends (account announcements, outage alert). `None` in unit tests.
    pub outbound: Option<Weak<ChannelRegistry>>,
}

impl CommandContext {
    pub fn channel(&self) -> Channel {
        self.sender.channel()
    }

    /// Storage-form conversation, for the existing `room_id` columns.
    pub fn room_key(&self) -> String {
        self.conversation.to_storage()
    }

    /// The sender's MXID when the command came over Matrix.
    pub fn sender_mxid(&self) -> Option<&str> {
        (self.channel() == Channel::Matrix).then(|| self.sender.user_id().as_str())
    }

    /// Display name for "who typed this" — never an ownership key.
    pub fn sender_name(&self) -> &str {
        display_name_of(&self.sender)
    }
}

/// What an image or voice turn produced: the HTML reply and the tool calls the
/// turn ran, `(name, result)`. The adapter that showed the progress records the
/// tool targets against its own progress message; the turn knows no channel.
pub struct MediaReply {
    pub html: String,
    pub tool_results: Vec<(String, String)>,
}

impl From<String> for MediaReply {
    /// A reply with no tool calls: every refusal and failure notice.
    fn from(html: String) -> Self {
        Self {
            html,
            tool_results: Vec::new(),
        }
    }
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
/// Each module handles a set of commands; background work belongs to the job
/// and events runtimes, not to a module.
#[async_trait]
pub trait BotModule: Send + Sync {
    /// Returns the module's display name (e.g. "Core", "To-Do", "Reminder").
    fn name(&self) -> &str;

    /// Returns a list of commands this module handles, for help text generation.
    fn commands(&self) -> Vec<CommandHelp>;

    /// Attempts to handle a command. Returns `Some(reply)` if handled, `None` to pass.
    ///
    /// A module that mutates state does so through `services`; the service
    /// publishes its `SystemEvent` through the outbox in the same transaction,
    /// so a change made over Matrix reaches every open web client without
    /// the module doing anything.
    ///
    /// `services` is the application layer built once in `app.rs`; a moved
    /// feature's command handler calls its service through it. Modules not
    /// yet moved ignore it.
    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        db: &Database,
    ) -> Option<Reply>;
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
    ) -> Option<Reply> {
        for module in &self.modules {
            if let Some(reply) = module.handle_command(ctx, services, db).await {
                return Some(reply);
            }
        }
        None
    }

    /// Returns help text for all modules, grouped by module name.
    pub fn get_all_help(&self) -> Vec<(String, Vec<CommandHelp>)> {
        self.modules
            .iter()
            .map(|m| (m.name().to_string(), m.commands()))
            .collect()
    }
}

/// What a failed use case tells the user in chat: the chat counterpart of
/// `interface/http/app_error.rs`, and the only place a command handler
/// turns an `AppError` into a reply.
///
/// Returns the reply a `BotModule` hands back. The four user-facing
/// variants show their (escaped) message; a database or external failure is
/// logged here and answered with a fixed apology, so a SQL or provider error
/// never reaches a room.
pub fn error_reply(err: AppError) -> Reply {
    let message = match err {
        AppError::Validation(msg)
        | AppError::NotFound(msg)
        | AppError::Forbidden(msg)
        | AppError::Conflict(msg) => msg,
        AppError::Database(err) => {
            error!(error = %err, "Chat command failed on the database");
            return Reply::html(INTERNAL_REPLY);
        }
        AppError::External { service, source } => {
            error!(service, error = %format!("{source:#}"), "Chat command failed on a dependency");
            return Reply::html(INTERNAL_REPLY);
        }
    };
    Reply::html(format!("⚠️ {}", html_escape(&message)))
}

const INTERNAL_REPLY: &str = "⚠️ Something went wrong on our side. Please try again later.";

/// Context builders shared by the modules' tests.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// A direct Matrix context: room `!room:a`, sender `@tester:x` shown as
    /// `alice`, owner seeded from `@alice:a`.
    pub fn context(command: &str, raw_args: &str, full_body: &str) -> CommandContext {
        context_for("@tester:x", "alice", command, raw_args, full_body)
    }

    pub fn sender(mxid: &str, name: &str) -> PlatformIdentity {
        PlatformIdentity::new(
            Channel::Matrix,
            crate::domain::channels::PlatformUserId::parse(Channel::Matrix, mxid)
                .expect("test mxid"),
            Some(name.to_owned()),
        )
    }

    pub fn context_for(
        mxid: &str,
        name: &str,
        command: &str,
        raw_args: &str,
        full_body: &str,
    ) -> CommandContext {
        let sender = sender(mxid, name);
        CommandContext {
            conversation: ConversationAddress::Matrix("!room:a".into()),
            owner_id: AccountId::from_seed("@alice:a"),
            sender,
            command: command.to_owned(),
            raw_args: raw_args.to_owned(),
            full_body: full_body.to_owned(),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            progress: None,
            outbound: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity};

    #[test]
    fn user_facing_errors_show_their_escaped_message() {
        let reply = error_reply(AppError::Validation("<b>too long</b>".into()));
        assert_eq!(reply.html_body(), "⚠️ &lt;b&gt;too long&lt;/b&gt;");
        assert_eq!(
            error_reply(AppError::NotFound("No such todo".into())).html_body(),
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
        assert_eq!(db.html_body(), INTERNAL_REPLY);
        assert_eq!(ext.html_body(), INTERNAL_REPLY);
    }

    #[test]
    fn a_matrix_context_exposes_the_same_room_key_and_mxid_as_before() {
        let ctx = ctx_for("todo");
        assert_eq!(ctx.room_key(), "!room:a");
        assert_eq!(ctx.sender_mxid(), Some("@tester:x"));
        assert_eq!(ctx.channel(), Channel::Matrix);
    }

    #[test]
    fn error_replies_keep_their_wording() {
        assert_eq!(
            error_reply(AppError::Validation("<b>too long</b>".into())).html_body(),
            "⚠️ &lt;b&gt;too long&lt;/b&gt;"
        );
    }

    /// Without a display name the sender is shown by the MXID localpart, the
    /// value the Matrix parser has always put in `sender_name`.
    #[test]
    fn the_sender_name_falls_back_to_the_mxid_localpart() {
        let parsed = ParsedCommand {
            conversation: ConversationAddress::Matrix("!room:a".into()),
            sender: PlatformIdentity::matrix("@tester:x").unwrap(),
            command: "todo".into(),
            raw_args: String::new(),
            full_body: "!todo".into(),
            is_mentioned: false,
            quoted: None,
        };
        let ctx = parsed.for_owner(AccountId::from_seed("@alice:a"), true);
        assert_eq!(ctx.sender_name(), "tester");
        assert!(ctx.progress.is_none() && ctx.outbound.is_none());
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
            let ctx = test_support::context(
                body.split_whitespace()
                    .next()
                    .unwrap()
                    .trim_start_matches('!'),
                "",
                body,
            );
            assert!(
                registry
                    .dispatch(&ctx, &crate::application::AppServices::new(&db.pool), &db)
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
            ) -> Option<Reply> {
                ctx.is_direct
                    .then(|| Reply::from("answered by the catch-all".to_string()))
            }
        }

        let db = memory_db().await;
        let tts_module = || {
            TtsModule::new(
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
            )
            .await
            .expect("TTS claims `say`");
        assert!(
            !reply.html_body().contains("catch-all"),
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
            )
            .await
            .expect("the catch-all claims every direct message");
        assert!(
            reply.html_body().contains("catch-all"),
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
        ) -> Option<Reply> {
            (ctx.command == self.claims).then(|| Reply::from(format!("handled by {}", self.label)))
        }
    }

    fn ctx_for(command: &str) -> CommandContext {
        test_support::context(command, "", &format!("!{}", command))
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
            )
            .await;
        assert_eq!(reply.unwrap().html_body(), "handled by first");
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
                &db
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
            )
            .await;
        assert_eq!(reply.unwrap().html_body(), "handled by finance");
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
