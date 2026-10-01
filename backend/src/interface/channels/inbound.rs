//! The inbound pipeline every channel shares (spec §5, steps 2-4).
//!
//! An adapter parses its platform's syntax (step 1) into an [`InboundText`] and
//! sends the reply (step 5) so it can attach its own progress UI. Everything in
//! between is channel-neutral and lives here: `link` before identity
//! resolution, identity resolution under the channel's provisioning policy,
//! free text becoming the `ai` command, and the panic fence around dispatch.

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Weak};

use futures_util::FutureExt;
use tracing::{error, warn};

use super::message::{IncomingMessage, Reply};
use super::registry::ChannelRegistry;
use crate::application::identity::Resolution;
use crate::application::AppServices;
use crate::db::Database;
use crate::interface::chat::{ModuleRegistry, ParsedCommand};
use crate::llm::progress::ProgressSink;

pub const UNLINKED_REPLY: &str = "This account is not linked yet. Send <code>/link</code> in a direct message to connect it to your workspace.";

/// Appended to [`UNLINKED_REPLY`] for `start` and `help`, the two commands a
/// stranger sends first.
const UNLINKED_HINT: &str = "Commands available once linked: todo, expense, portfolio, ask.";

/// What a user sees when a module panicked; the text Matrix has always sent.
pub const COMMAND_PANIC_REPLY: &str = "⚠️ <b>指令处理遇到异常</b><br/>• <b>原因:</b> 系统处理请求时发生意外错误<br/>• <b>建议:</b> 请稍后重试，或检查输入内容。";

/// The adapter's parse of one message.
pub enum InboundText {
    Command {
        command: String,
        raw_args: String,
        full_body: String,
    },
    /// Not a command; becomes an `ai` prompt where the bot is addressed.
    FreeText(String),
}

pub enum InboundOutcome {
    Replied(Reply),
    Ignored,
}

pub struct Inbound {
    services: AppServices,
    db: Database,
    modules: Arc<ModuleRegistry>,
    /// Weak: adapters own the pipeline and the registry owns the adapters (A5).
    registry: Weak<ChannelRegistry>,
}

impl Inbound {
    pub fn new(
        services: AppServices,
        db: Database,
        modules: Arc<ModuleRegistry>,
        registry: Weak<ChannelRegistry>,
    ) -> Self {
        Self {
            services,
            db,
            modules,
            registry,
        }
    }

    pub async fn handle(
        &self,
        msg: &IncomingMessage,
        text: InboundText,
        progress: Option<Arc<dyn ProgressSink>>,
    ) -> InboundOutcome {
        // `link` precedes identity resolution, and that ordering is
        // load-bearing: resolving first would provision an empty account for
        // an unknown sender and turn every first link into a merge of nothing.
        if let InboundText::Command { command, .. } = &text {
            if command.eq_ignore_ascii_case("link") {
                return InboundOutcome::Replied(
                    crate::interface::chat::link::handle_link(&self.services, msg).await,
                );
            }
        }

        let owner = match self.services.identity.resolve(&msg.sender).await {
            Ok(Resolution::Account(owner)) => owner,
            Ok(Resolution::Unlinked) => return unlinked_outcome(msg, &text),
            Err(err) => {
                warn!(channel = %msg.channel, error = %err, "could not resolve the sender; dropping the message");
                return InboundOutcome::Ignored;
            }
        };

        let Some((command, raw_args, full_body)) = into_command(msg, text) else {
            return InboundOutcome::Ignored;
        };
        let mut ctx = ParsedCommand {
            conversation: msg.conversation.clone(),
            sender: msg.sender.clone(),
            command,
            raw_args,
            full_body,
            is_mentioned: msg.is_mentioned,
            quoted: msg.quoted.clone(),
        }
        .for_owner(owner, msg.is_direct);
        ctx.progress = progress;
        ctx.outbound = Some(self.registry.clone());

        let dispatched = AssertUnwindSafe(self.modules.dispatch(&ctx, &self.services, &self.db))
            .catch_unwind()
            .await;
        match dispatched {
            Ok(Some(reply)) => InboundOutcome::Replied(reply),
            Ok(None) => InboundOutcome::Ignored,
            Err(panic) => {
                error!(channel = %msg.channel, "Command dispatch panicked: {panic:?}");
                InboundOutcome::Replied(Reply::html(COMMAND_PANIC_REPLY))
            }
        }
    }
}

/// An unlinked sender has no account, so nothing is dispatched for them.
fn unlinked_outcome(msg: &IncomingMessage, text: &InboundText) -> InboundOutcome {
    match text {
        InboundText::Command { command, .. }
            if command.eq_ignore_ascii_case("start") || command.eq_ignore_ascii_case("help") =>
        {
            InboundOutcome::Replied(Reply::html(format!("{UNLINKED_REPLY}<br/>{UNLINKED_HINT}")))
        }
        InboundText::Command { .. } => InboundOutcome::Replied(Reply::html(UNLINKED_REPLY)),
        InboundText::FreeText(_) if msg.is_direct => {
            InboundOutcome::Replied(Reply::html(UNLINKED_REPLY))
        }
        InboundText::FreeText(_) => InboundOutcome::Ignored,
    }
}

/// `(command, raw_args, full_body)`: free text becomes an `ai` prompt where the
/// bot is addressed (a direct conversation or a mention), else nothing.
fn into_command(msg: &IncomingMessage, text: InboundText) -> Option<(String, String, String)> {
    match text {
        InboundText::Command {
            command,
            raw_args,
            full_body,
        } => Some((command, raw_args, full_body)),
        InboundText::FreeText(prompt) => {
            let addressed = msg.is_direct || msg.is_mentioned;
            (addressed && !prompt.trim().is_empty())
                .then(|| ("ai".to_owned(), prompt.clone(), prompt))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;

    use super::*;
    use crate::application::AppServices;
    use crate::db::{Database, DbPool};
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::{Channel, ConversationAddress, PlatformIdentity, PlatformUserId};
    use crate::infrastructure::postgres::repositories::accounts::core;
    use crate::infrastructure::postgres::repositories::identities;
    use crate::infrastructure::postgres::repositories::link_tokens;
    use crate::interface::channels::fake::FakeAdapter;
    use crate::interface::channels::message::{IncomingMessage, MessageRef, Reply};
    use crate::interface::channels::registry::ChannelRegistry;
    use crate::interface::chat::{BotModule, CommandContext, CommandHelp, ModuleRegistry};

    /// Claims one command and answers "handled by <command>".
    struct StubModule(&'static str);

    impl StubModule {
        fn claiming(command: &'static str) -> Self {
            Self(command)
        }
    }

    #[async_trait]
    impl BotModule for StubModule {
        fn name(&self) -> &str {
            "stub"
        }
        fn commands(&self) -> Vec<CommandHelp> {
            Vec::new()
        }
        async fn handle_command(
            &self,
            ctx: &CommandContext,
            _services: &AppServices,
            _db: &Database,
        ) -> Option<Reply> {
            (ctx.command == self.0).then(|| Reply::html(format!("handled by {}", ctx.command)))
        }
    }

    struct PanickingModule;

    #[async_trait]
    impl BotModule for PanickingModule {
        fn name(&self) -> &str {
            "panics"
        }
        fn commands(&self) -> Vec<CommandHelp> {
            Vec::new()
        }
        async fn handle_command(
            &self,
            _ctx: &CommandContext,
            _services: &AppServices,
            _db: &Database,
        ) -> Option<Reply> {
            panic!("module bug")
        }
    }

    /// The registry is kept alive by the returned `Arc`; `Inbound` holds only a `Weak` (A5).
    async fn pipeline_with(
        module: impl BotModule + 'static,
    ) -> (Inbound, DbPool, Arc<ChannelRegistry>) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let mut modules = ModuleRegistry::new();
        modules.register(module);
        let registry = Arc::new(
            ChannelRegistry::new(vec![
                Arc::new(FakeAdapter::connected(Channel::Telegram)),
                Arc::new(FakeAdapter::connected(Channel::Matrix)),
            ])
            .expect("one adapter per channel"),
        );
        let inbound = Inbound::new(
            AppServices::new(&pool),
            Database { pool: pool.clone() },
            Arc::new(modules),
            Arc::downgrade(&registry),
        );
        (inbound, pool, registry)
    }

    fn ident(channel: Channel, raw: &str) -> PlatformIdentity {
        PlatformIdentity::new(channel, PlatformUserId::parse(channel, raw).unwrap(), None)
    }
    fn tg(raw: &str) -> PlatformIdentity {
        ident(Channel::Telegram, raw)
    }
    fn mx(raw: &str) -> PlatformIdentity {
        ident(Channel::Matrix, raw)
    }

    fn incoming(
        channel: Channel,
        conversation: ConversationAddress,
        sender: PlatformIdentity,
        is_direct: bool,
    ) -> IncomingMessage {
        IncomingMessage {
            channel,
            conversation,
            sender,
            message_id: MessageRef("m1".into()),
            text: String::new(),
            reply_to: None,
            quoted: None,
            is_direct,
            is_mentioned: false,
        }
    }

    fn cmd(command: &str, args: &str) -> InboundText {
        InboundText::Command {
            command: command.into(),
            raw_args: args.into(),
            full_body: format!("/{command} {args}"),
        }
    }

    async fn create_account(pool: &DbPool) -> crate::domain::accounts::AccountId {
        core::create(pool, "a", STATUS_ACTIVE).await.unwrap()
    }

    fn assert_reply_contains(out: InboundOutcome, needle: &str) {
        match out {
            InboundOutcome::Replied(r) => {
                assert!(
                    r.content.plain.contains(needle) || r.html_body().contains(needle),
                    "{:?} lacks {needle}",
                    r.content.plain
                )
            }
            InboundOutcome::Ignored => panic!("expected a reply containing {needle}, got Ignored"),
        }
    }

    #[tokio::test]
    async fn an_unlinked_telegram_user_reaches_no_module() {
        let (inbound, _pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(42),
            tg("987654321"),
            true,
        );
        let out = inbound.handle(&msg, cmd("todo", "add milk"), None).await;
        assert_reply_contains(out, "not linked yet");
    }

    #[tokio::test]
    async fn a_linked_telegram_user_reaches_the_same_module_as_matrix() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let a = create_account(&pool).await;
        identities::bind(&pool, &a, &tg("987654321")).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(42),
            tg("987654321"),
            true,
        );
        let out = inbound.handle(&msg, cmd("todo", "list"), None).await;
        assert_reply_contains(out, "handled by todo");
    }

    #[tokio::test]
    async fn free_text_in_a_direct_conversation_goes_to_the_ai_command() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("ai")).await;
        let a = create_account(&pool).await;
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(1),
            tg("1"),
            true,
        );
        let out = inbound
            .handle(&msg, InboundText::FreeText("hello".into()), None)
            .await;
        assert_reply_contains(out, "handled by ai");
    }

    #[tokio::test]
    async fn free_text_in_a_group_without_a_mention_is_ignored() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("ai")).await;
        let a = create_account(&pool).await;
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(-5),
            tg("1"),
            false,
        );
        let out = inbound
            .handle(&msg, InboundText::FreeText("hello".into()), None)
            .await;
        assert!(matches!(out, InboundOutcome::Ignored));
    }

    #[tokio::test]
    async fn free_text_in_a_group_with_a_mention_goes_to_the_ai_command() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("ai")).await;
        let a = create_account(&pool).await;
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        let mut msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(-5),
            tg("1"),
            false,
        );
        msg.is_mentioned = true;
        let out = inbound
            .handle(&msg, InboundText::FreeText("hello".into()), None)
            .await;
        assert_reply_contains(out, "handled by ai");
    }

    #[tokio::test]
    async fn unlinked_free_text_is_refused_in_a_dm_and_ignored_in_a_group() {
        let (inbound, _pool, _reg) = pipeline_with(StubModule::claiming("ai")).await;
        let dm = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(7),
            tg("7"),
            true,
        );
        assert_reply_contains(
            inbound
                .handle(&dm, InboundText::FreeText("hi".into()), None)
                .await,
            "not linked yet",
        );
        let group = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(-7),
            tg("7"),
            false,
        );
        assert!(matches!(
            inbound
                .handle(&group, InboundText::FreeText("hi".into()), None)
                .await,
            InboundOutcome::Ignored
        ));
    }

    #[tokio::test]
    async fn a_matrix_stranger_is_provisioned_and_dispatched() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let msg = incoming(
            Channel::Matrix,
            ConversationAddress::Matrix("!r:x".into()),
            mx("@new:x"),
            true,
        );
        let out = inbound.handle(&msg, cmd("todo", "list"), None).await;
        assert_reply_contains(out, "handled by todo");
        let owner = crate::application::AppServices::new(&pool)
            .identity
            .find_account(&mx("@new:x"))
            .await
            .unwrap();
        assert!(owner.is_some(), "the stranger now resolves to an account");
    }

    #[tokio::test]
    async fn a_panicking_module_gets_the_fixed_apology() {
        let (inbound, pool, _reg) = pipeline_with(PanickingModule).await;
        let a = create_account(&pool).await;
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(1),
            tg("1"),
            true,
        );
        match inbound.handle(&msg, cmd("todo", "x"), None).await {
            InboundOutcome::Replied(r) => assert_eq!(r.html_body(), COMMAND_PANIC_REPLY),
            InboundOutcome::Ignored => panic!("a panic must be answered"),
        }
    }

    #[tokio::test]
    async fn a_command_no_module_claims_is_ignored() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let a = create_account(&pool).await;
        identities::bind(&pool, &a, &tg("1")).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(1),
            tg("1"),
            true,
        );
        assert!(matches!(
            inbound.handle(&msg, cmd("nothing", ""), None).await,
            InboundOutcome::Ignored
        ));
    }

    #[tokio::test]
    async fn link_from_an_unlinked_telegram_identity_does_not_provision() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let before = core::count(&pool).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(9),
            tg("9"),
            true,
        );
        let _ = inbound.handle(&msg, cmd("link", "ABC123"), None).await;
        assert_eq!(
            core::count(&pool).await.unwrap(),
            before,
            "link must never provision"
        );
    }

    #[tokio::test]
    async fn start_for_an_unlinked_identity_lists_what_unlocks_after_linking() {
        let (inbound, _pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(8),
            tg("8"),
            true,
        );
        for command in ["start", "help"] {
            let out = inbound.handle(&msg, cmd(command, ""), None).await;
            assert_reply_contains(out, "not linked yet");
            let out = inbound.handle(&msg, cmd(command, ""), None).await;
            assert_reply_contains(
                out,
                "Commands available once linked: todo, expense, portfolio, ask.",
            );
        }
    }
    #[tokio::test]
    async fn link_in_a_direct_message_from_an_unlinked_telegram_identity_mints_a_token() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let before = core::count(&pool).await.unwrap();
        let msg = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(9),
            tg("9"),
            true,
        );
        let out = inbound.handle(&msg, cmd("link", ""), None).await;
        assert_reply_contains(out, "/link?token=");
        assert_eq!(core::count(&pool).await.unwrap(), before);
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn link_in_a_group_is_refused_for_every_channel_and_mints_nothing() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let telegram = incoming(
            Channel::Telegram,
            ConversationAddress::Telegram(-9),
            tg("9"),
            false,
        );
        assert_reply_contains(
            inbound.handle(&telegram, cmd("link", ""), None).await,
            "direct message",
        );
        let matrix = incoming(
            Channel::Matrix,
            ConversationAddress::Matrix("!room:x".into()),
            mx("@evan:mbm.one"),
            false,
        );
        assert_reply_contains(
            inbound.handle(&matrix, cmd("link", ""), None).await,
            "direct message",
        );
        assert_eq!(link_tokens::count(&pool).await.unwrap(), 0);
    }

    /// The old `!link <code>` accepted a code from the web app; now any
    /// argument is ignored and the sender is not provisioned either.
    #[tokio::test]
    async fn matrix_link_ignores_arguments_and_does_not_provision() {
        let (inbound, pool, _reg) = pipeline_with(StubModule::claiming("todo")).await;
        let before = core::count(&pool).await.unwrap();
        let msg = incoming(
            Channel::Matrix,
            ConversationAddress::Matrix("!dm:x".into()),
            mx("@evan:mbm.one"),
            true,
        );
        let out = inbound.handle(&msg, cmd("link", "ABCD-1234"), None).await;
        assert_reply_contains(out, "/link?token=");
        assert_eq!(core::count(&pool).await.unwrap(), before);
    }
}
