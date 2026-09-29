//! Investment holdings: symbol, quantity, and weighted-average cost basis.

use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::interface::chat::{BotModule, CommandContext, CommandHelp, ModuleContext};

pub mod alerts_handler;
pub mod cash;
pub mod handler;
pub mod scheduler;
pub mod trade_cmd;
pub mod view;

pub struct PortfolioModule;

impl Default for PortfolioModule {
    fn default() -> Self {
        Self::new()
    }
}

impl PortfolioModule {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BotModule for PortfolioModule {
    fn name(&self) -> &str {
        "Portfolio"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "portfolio buy".to_string(),
                usage: "!portfolio buy <symbol> <qty> @ <price> [CUR] [note]".to_string(),
                description: "Open or add to a position using cash \
                              (e.g. !portfolio buy AAPL 10 @ 150.00 USD core position)"
                    .to_string(),
            },
            CommandHelp {
                name: "portfolio record".to_string(),
                usage: "!portfolio record <symbol> <qty> @ <price> [CUR] [note]".to_string(),
                description: "Record already-owned position without spending cash \
                              (e.g. !portfolio record BTC 0.5 @ 60000 USD cold storage)"
                    .to_string(),
            },
            CommandHelp {
                name: "portfolio sell".to_string(),
                usage: "!portfolio sell <symbol> <qty> @ <price>".to_string(),
                description: "Reduce or close a position; always reports realized profit/loss"
                    .to_string(),
            },
            CommandHelp {
                name: "portfolio price".to_string(),
                usage: "!portfolio price <symbol> <price>".to_string(),
                description: "Record the current market price for a holding".to_string(),
            },
            CommandHelp {
                name: "portfolio refresh".to_string(),
                usage: "!portfolio refresh".to_string(),
                description:
                    "Automatically fetch and update current market prices for all holdings"
                        .to_string(),
            },
            CommandHelp {
                name: "portfolio note".to_string(),
                usage: "!portfolio note <symbol> [text]".to_string(),
                description: "Set a note on a holding, or clear it if no text is given".to_string(),
            },
            CommandHelp {
                name: "portfolio cash".to_string(),
                usage: "!portfolio cash".to_string(),
                description: "Show your cash balance(s), per currency".to_string(),
            },
            CommandHelp {
                name: "portfolio cash deposit".to_string(),
                usage: "!portfolio cash deposit <amount> [CUR]".to_string(),
                description: "Add cash — buying requires enough of it on hand".to_string(),
            },
            CommandHelp {
                name: "portfolio cash withdraw".to_string(),
                usage: "!portfolio cash withdraw <amount> [CUR]".to_string(),
                description: "Remove cash; rejected if it would go negative".to_string(),
            },
            CommandHelp {
                name: "portfolio list".to_string(),
                usage: "!portfolio [list]".to_string(),
                description: "Show your holdings with current value and unrealized P&L".to_string(),
            },
            CommandHelp {
                name: "portfolio summary".to_string(),
                usage: "!portfolio summary".to_string(),
                description: "Per-currency totals: cost basis, market value, unrealized P&L"
                    .to_string(),
            },
            CommandHelp {
                name: "portfolio remove".to_string(),
                usage: "!portfolio remove <symbol>".to_string(),
                description: "Remove a holding entirely".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &crate::application::AppServices,
        _db: &Database,
        _broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_portfolio_command(services, &ctx.owner_id, &ctx.full_body).await
    }

    async fn spawn_background_workers(
        &self,
        ctx: &ModuleContext,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        vec![scheduler::start_portfolio_price_scheduler(ctx.clone())]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::chat::CommandContext;

    fn ctx(body: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            sender_mxid: "@tester:x".to_string(),
            owner_id: crate::domain::accounts::AccountId::new("@alice:a"),
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
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn db() -> Database {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        Database { pool }
    }

    async fn fund(
        module: &PortfolioModule,
        db: &Database,
        services: &crate::application::AppServices,
        currency: &str,
    ) {
        module
            .handle_command(
                &ctx(&format!("!portfolio cash deposit 100000 {currency}")),
                services,
                db,
                None,
            )
            .await
            .unwrap();
    }

    #[test]
    fn test_advertises_its_commands() {
        let module = PortfolioModule::new();
        assert_eq!(module.name(), "Portfolio");

        let commands = module.commands();
        assert!(commands.iter().any(|c| c.name == "portfolio buy"));
        assert!(commands.iter().any(|c| c.name == "portfolio sell"));
        assert!(commands
            .iter()
            .all(|c| !c.usage.is_empty() && !c.description.is_empty()));
    }

    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);
        fund(&module, &db, &services, "USD").await;

        assert!(module
            .handle_command(&ctx("!portfolio"), &services, &db, None)
            .await
            .is_some());
        assert!(module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00"),
                &services,
                &db,
                None
            )
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!portfolio cash"), &services, &db, None)
            .await
            .is_some());

        assert!(module
            .handle_command(&ctx("!todo add milk"), &services, &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!expense add 1 EUR x"), &services, &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!settings"), &services, &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("hello there"), &services, &db, None)
            .await
            .is_none());
    }

    #[tokio::test]
    async fn test_buy_then_list() {
        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);
        fund(&module, &db, &services, "USD").await;

        let reply = module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00 USD"),
                &services,
                &db,
                None,
            )
            .await
            .expect("the module must answer");
        assert!(reply.contains("AAPL"));
        assert!(
            reply.contains("Cash remaining: 98500.00 USD"),
            "should state the cash left: {reply}"
        );

        let listed = module
            .handle_command(&ctx("!portfolio list"), &services, &db, None)
            .await
            .unwrap();
        assert!(
            listed.contains("AAPL"),
            "the position should be listed: {listed}"
        );
    }

    #[tokio::test]
    async fn test_buy_without_enough_cash_is_rejected() {
        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        let reply = module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        assert!(
            reply.contains("Need"),
            "should explain the shortfall: {reply}"
        );

        let listed = module
            .handle_command(&ctx("!portfolio list"), &services, &db, None)
            .await
            .unwrap();
        assert!(
            listed.contains("No holdings yet"),
            "a rejected buy must not create a position: {listed}"
        );
    }

    #[tokio::test]
    async fn test_buy_without_a_currency_uses_the_senders_default() {
        use crate::domain::settings::UserSettings;

        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        services
            .settings
            .save(
                &crate::domain::accounts::AccountId::new("@alice:a"),
                UserSettings {
                    timezone: "UTC".to_string(),
                    currency: "CNY".to_string(),
                    ..UserSettings::default()
                },
            )
            .await
            .unwrap();
        fund(&module, &db, &services, "CNY").await;

        let reply = module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        assert!(
            reply.contains("CNY"),
            "should use the saved currency: {reply}"
        );
    }

    #[tokio::test]
    async fn test_buy_broadcasts_holding_and_cash_to_the_owner() {
        let module = PortfolioModule::new();
        let db = db().await;
        let broadcaster = EventBroadcaster::default();
        let services = crate::application::AppServices::with_events(&db.pool, &broadcaster);
        fund(&module, &db, &services, "USD").await;
        let mut rx = broadcaster.subscribe();

        module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00 USD"),
                &services,
                &db,
                Some(&broadcaster),
            )
            .await
            .unwrap();

        let mut saw_holding = false;
        let mut saw_cash = false;
        while let Ok(event) = rx.try_recv() {
            assert_eq!(
                event.owner_id(),
                crate::domain::accounts::AccountId::from("@alice:a").to_string()
            );
            match event {
                crate::events::SystemEvent::PortfolioHoldingCreated(_) => saw_holding = true,
                crate::events::SystemEvent::PortfolioCashUpdated { balance_minor, .. } => {
                    saw_cash = true;
                    assert_eq!(balance_minor, 10_000_000 - 150_000);
                }
                _ => {}
            }
        }
        assert!(saw_holding, "a buy must broadcast the holding");
        assert!(saw_cash, "a buy must broadcast the debited cash");
    }

    #[tokio::test]
    async fn test_list_does_not_broadcast() {
        let module = PortfolioModule::new();
        let db = db().await;
        let broadcaster = EventBroadcaster::default();
        let services = crate::application::AppServices::with_events(&db.pool, &broadcaster);
        let mut rx = broadcaster.subscribe();

        module
            .handle_command(&ctx("!portfolio list"), &services, &db, Some(&broadcaster))
            .await
            .unwrap();
        module
            .handle_command(&ctx("!portfolio cash"), &services, &db, Some(&broadcaster))
            .await
            .unwrap();

        assert!(rx.try_recv().is_err(), "a read must not broadcast");
    }

    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);
        fund(&module, &db, &services, "USD").await;

        module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();

        let note_reply = module
            .handle_command(
                &ctx("!portfolio note AAPL <script>alert(1)</script>"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        assert!(
            !note_reply.contains("<script>"),
            "raw markup must not survive: {note_reply}"
        );
        assert!(note_reply.contains("&lt;script&gt;"));

        let listed = module
            .handle_command(&ctx("!portfolio list"), &services, &db, None)
            .await
            .unwrap();
        assert!(
            !listed.contains("<script>"),
            "raw markup must not survive: {listed}"
        );
        assert!(listed.contains("&lt;script&gt;"));
    }

    #[tokio::test]
    async fn test_cash_deposit_buy_sell_withdraw_round_trip() {
        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);

        let deposited = module
            .handle_command(
                &ctx("!portfolio cash deposit 1000 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        assert!(deposited.contains("1000.00"), "{deposited}");

        module
            .handle_command(
                &ctx("!portfolio buy AAPL 5 @ 150.00 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        let after_buy = module
            .handle_command(&ctx("!portfolio cash"), &services, &db, None)
            .await
            .unwrap();
        assert!(after_buy.contains("250.00"), "{after_buy}");

        module
            .handle_command(
                &ctx("!portfolio sell AAPL 5 @ 180.00"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        let after_sell = module
            .handle_command(&ctx("!portfolio cash"), &services, &db, None)
            .await
            .unwrap();
        assert!(after_sell.contains("1150.00"), "{after_sell}");

        let withdrawn = module
            .handle_command(
                &ctx("!portfolio cash withdraw 1150 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        assert!(withdrawn.contains("0.00"), "{withdrawn}");

        // Verify deposit and withdraw transactions were posted to the finance ledger
        let txs = crate::infrastructure::postgres::repositories::finance::list(
            &db.pool,
            &crate::domain::accounts::AccountId::new("@alice:a"),
            &crate::domain::finance::ListTransactionsFilter {
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].direction, crate::domain::finance::Direction::Income);
        assert_eq!(txs[0].amount_minor, 115000);
        assert_eq!(txs[0].source_kind.as_deref(), Some("portfolio_cash"));
        assert_eq!(txs[1].direction, crate::domain::finance::Direction::Expense);
        assert_eq!(txs[1].amount_minor, 100000);
        assert_eq!(txs[1].source_kind.as_deref(), Some("portfolio_cash"));
    }

    #[tokio::test]
    async fn test_cash_withdraw_insufficient_funds() {
        let module = PortfolioModule::new();
        let db = db().await;
        let services = crate::application::AppServices::new(&db.pool);
        module
            .handle_command(
                &ctx("!portfolio cash deposit 100 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();

        let reply = module
            .handle_command(
                &ctx("!portfolio cash withdraw 101 USD"),
                &services,
                &db,
                None,
            )
            .await
            .unwrap();
        assert!(
            reply.contains("100.00"),
            "should state what is actually available: {reply}"
        );
    }
}
