use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp};

pub mod handler;
pub mod models;
pub mod money;
pub mod repository;

pub struct FinanceModule;

impl Default for FinanceModule {
    fn default() -> Self {
        Self::new()
    }
}

impl FinanceModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for FinanceModule {
    fn name(&self) -> &str {
        "Finance"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "expense add".to_string(),
                usage: "!expense add <amount> [CUR] <note> [-c <category>]".to_string(),
                description: "Log spending (e.g. !expense add 15.50 EUR lunch -c food)".to_string(),
            },
            CommandHelp {
                name: "income add".to_string(),
                usage: "!income add <amount> [CUR] <note> [-c <category>]".to_string(),
                description: "Log money received (e.g. !income add 2500 EUR salary)".to_string(),
            },
            CommandHelp {
                name: "expense list".to_string(),
                usage: "!expense list [page]".to_string(),
                description: "Show recent ledger entries".to_string(),
            },
            CommandHelp {
                name: "expense summary".to_string(),
                usage: "!expense summary".to_string(),
                description: "Totals for this month, grouped per currency".to_string(),
            },
            CommandHelp {
                name: "expense delete".to_string(),
                usage: "!expense delete <id> / !expense rm <id>".to_string(),
                description: "Remove a ledger entry".to_string(),
            },
            CommandHelp {
                name: "finance".to_string(),
                usage: "!finance [summary|list]".to_string(),
                description: "View financial ledger and summary".to_string(),
            },
            CommandHelp {
                name: "category".to_string(),
                usage: "!category [add|list|remove] <name>".to_string(),
                description: "Manage your spending categories".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_finance_command(
            &db.pool,
            &ctx.room_id,
            &ctx.owner_id,
            &ctx.sender_name,
            &ctx.full_body,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::modules::CommandContext;

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@tester:x".to_string(),
            owner_id: "@alice:a".to_string(),
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
        }
    }

    async fn db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
    }

    #[test]
    fn test_advertises_its_commands_for_help() {
        let module = FinanceModule::new();
        assert_eq!(module.name(), "Finance");

        let commands = module.commands();
        assert!(!commands.is_empty());
        assert!(commands.iter().any(|c| c.name == "expense add"));
        assert!(commands.iter().any(|c| c.name == "income add"));
        assert!(commands.iter().any(|c| c.name == "category"));
        // Every advertised command needs usage text, or help is useless.
        assert!(commands
            .iter()
            .all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    /// The module must claim its own commands and nothing else, or it will
    /// swallow another module's traffic — dispatch returns the first match.
    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = FinanceModule::new();
        let db = db().await;

        assert!(module
            .handle_command(&ctx("!expense list"), &db, None)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!income list"), &db, None)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!category list"), &db, None)
            .await
            .is_some());

        assert!(module
            .handle_command(&ctx("!todo add milk"), &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!remind me later"), &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("hello there"), &db, None)
            .await
            .is_none());
    }

    /// An end-to-end pass through the module: log an entry, then read it back.
    #[tokio::test]
    async fn test_logs_and_lists_an_entry() {
        let module = FinanceModule::new();
        let db = db().await;

        let reply = module
            .handle_command(&ctx("!expense add 15.50 EUR lunch"), &db, None)
            .await
            .expect("the module must answer");
        assert!(
            reply.contains("15.50"),
            "reply should confirm the amount: {reply}"
        );
        assert!(reply.contains("EUR"));

        let listed = module
            .handle_command(&ctx("!expense list"), &db, None)
            .await
            .unwrap();
        assert!(
            listed.contains("lunch"),
            "the entry should be listed: {listed}"
        );
    }

    /// The README's documented shape, with no currency given: it must be
    /// accepted and answered, not silently ignored.
    #[tokio::test]
    async fn test_handles_add_without_a_currency() {
        let module = FinanceModule::new();
        let db = db().await;

        let reply = module
            .handle_command(&ctx("!expense add 15.50 lunch"), &db, None)
            .await
            .expect("the module must answer `!expense add 15.50 lunch`");

        assert!(
            reply.contains("15.50"),
            "reply should confirm the amount: {reply}"
        );
        assert!(
            reply.contains("USD"),
            "should fall back to the default currency: {reply}"
        );
        assert!(reply.contains("lunch"), "should keep the note: {reply}");
    }

    /// With a preference saved, an amount typed without a currency books in the
    /// currency the user actually spends in rather than the hardcoded USD.
    #[tokio::test]
    async fn test_add_without_a_currency_uses_the_senders_default() {
        use crate::modules::settings::{SettingsRepository, UserSettings};

        let module = FinanceModule::new();
        let db = db().await;

        SettingsRepository::upsert(
            &db.pool,
            "@alice:a",
            &UserSettings {
                timezone: "UTC".to_string(),
                currency: "CNY".to_string(),
                dev_room_id: None,
                max_history_turns: 10,
            },
        )
        .await
        .unwrap();

        let reply = module
            .handle_command(&ctx("!expense add 15.50 lunch"), &db, None)
            .await
            .expect("the module must answer `!expense add 15.50 lunch`");

        assert!(
            reply.contains("CNY"),
            "should use the saved currency: {reply}"
        );
        assert!(
            !reply.contains("USD"),
            "should not fall back to USD: {reply}"
        );

        // An explicit code still wins over the preference.
        let explicit = module
            .handle_command(&ctx("!expense add 9.99 EUR coffee"), &db, None)
            .await
            .unwrap();
        assert!(
            explicit.contains("EUR"),
            "an explicit currency must win: {explicit}"
        );
    }

    /// User text reaches an HTML message, so it has to be escaped.
    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let module = FinanceModule::new();
        let db = db().await;

        let reply = module
            .handle_command(
                &ctx("!expense add 1.00 EUR <script>alert(1)</script>"),
                &db,
                None,
            )
            .await
            .unwrap();

        assert!(
            !reply.contains("<script>"),
            "raw markup must not survive: {reply}"
        );
        assert!(reply.contains("&lt;script&gt;"));
    }
}
