use async_trait::async_trait;

use crate::interface::channels::message::Reply;

use crate::db::Database;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};

pub mod budget;
pub mod category;
pub mod handler;

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
                name: "budget".to_string(),
                usage: "!budget [YYYY-MM]".to_string(),
                description: "Monthly budget status with spend pace (e.g. !budget 2026-09)"
                    .to_string(),
            },
            CommandHelp {
                name: "budget set".to_string(),
                usage: "!budget set <amount> [CUR] [category]".to_string(),
                description: "Set or update a monthly limit; no category means overall".to_string(),
            },
            CommandHelp {
                name: "budget rm".to_string(),
                usage: "!budget rm [category] [CUR]".to_string(),
                description: "Remove a budget (CUR only needed with several currencies)"
                    .to_string(),
            },
            CommandHelp {
                name: "budget alerts".to_string(),
                usage: "!budget alerts <on|off> [category] [CUR]".to_string(),
                description: "Toggle the 80 % / 100 % alerts for one budget".to_string(),
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
        services: &crate::application::AppServices,
        _db: &Database,
    ) -> Option<Reply> {
        handler::handle_finance_command(
            services,
            &ctx.room_key(),
            &ctx.owner_id,
            ctx.sender_name(),
            &ctx.full_body,
        )
        .await
        .map(Reply::private)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::interface::chat::CommandContext;

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            conversation: crate::domain::channels::ConversationAddress::Matrix(
                "!room:a".to_string(),
            ),
            owner_id: crate::domain::accounts::AccountId::from_seed("@alice:a"),
            sender: crate::interface::chat::test_support::sender("@tester:x", "alice"),
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
            progress: None,
            outbound: None,
        }
    }

    async fn db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        Database { pool }
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
        assert!(commands
            .iter()
            .all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = FinanceModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        assert!(module
            .handle_command(&ctx("!expense list"), &services, &db)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!income list"), &services, &db)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!category list"), &services, &db)
            .await
            .is_some());

        assert!(module
            .handle_command(&ctx("!todo add milk"), &services, &db)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!remind me later"), &services, &db)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("hello there"), &services, &db)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_logs_and_lists_an_entry() {
        let module = FinanceModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        let reply = module
            .handle_command(&ctx("!expense add 15.50 EUR lunch"), &services, &db)
            .await
            .expect("the module must answer")
            .html_body()
            .to_owned();
        assert!(
            reply.contains("15.50"),
            "reply should confirm the amount: {reply}"
        );
        assert!(reply.contains("EUR"));

        let listed = module
            .handle_command(&ctx("!expense list"), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(
            listed.contains("lunch"),
            "the entry should be listed: {listed}"
        );
    }

    #[tokio::test]
    async fn test_handles_add_without_a_currency() {
        let module = FinanceModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        let reply = module
            .handle_command(&ctx("!expense add 15.50 lunch"), &services, &db)
            .await
            .expect("the module must answer `!expense add 15.50 lunch`")
            .html_body()
            .to_owned();

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

    #[tokio::test]
    async fn test_add_without_a_currency_uses_the_senders_default() {
        use crate::domain::settings::UserSettings;

        let module = FinanceModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        services
            .settings
            .save(
                &crate::domain::accounts::AccountId::from_seed("@alice:a"),
                UserSettings {
                    timezone: "UTC".to_string(),
                    currency: "CNY".to_string(),
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();

        let reply = module
            .handle_command(&ctx("!expense add 15.50 lunch"), &services, &db)
            .await
            .expect("the module must answer `!expense add 15.50 lunch`")
            .html_body()
            .to_owned();

        assert!(
            reply.contains("CNY"),
            "should use the saved currency: {reply}"
        );
        assert!(
            !reply.contains("USD"),
            "should not fall back to USD: {reply}"
        );

        let explicit = module
            .handle_command(&ctx("!expense add 9.99 EUR coffee"), &services, &db)
            .await
            .unwrap()
            .html_body()
            .to_owned();
        assert!(
            explicit.contains("EUR"),
            "an explicit currency must win: {explicit}"
        );
    }

    /// Given a chat command, When the module dispatches an add, Then the
    /// creation lands in the outbox.
    #[tokio::test]
    async fn test_the_module_command_writes_the_creation_to_the_outbox() {
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        let reply = FinanceModule::new()
            .handle_command(&ctx("!expense add 15.50 EUR lunch"), &services, &db)
            .await
            .expect("the add must be handled")
            .html_body()
            .to_owned();
        assert!(reply.contains("Logged"), "unexpected reply: {reply}");

        let owner = crate::domain::accounts::AccountId::from_seed("@alice:a");
        assert_eq!(
            crate::infrastructure::postgres::repositories::outbox::event_types_for(
                &db.pool, &owner
            )
            .await
            .unwrap(),
            vec!["TransactionCreated"]
        );
    }

    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let module = FinanceModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        let reply = module
            .handle_command(
                &ctx("!expense add 1.00 EUR <script>alert(1)</script>"),
                &services,
                &db,
            )
            .await
            .unwrap()
            .html_body()
            .to_owned();

        assert!(
            !reply.contains("<script>"),
            "raw markup must not survive: {reply}"
        );
        assert!(reply.contains("&lt;script&gt;"));
    }
}
