//! Investment holdings: symbol, quantity, and weighted-average cost basis.
//!
//! Manual entry only — there is no live price feed. `!portfolio price` and the
//! web UI's price field are how a holding's current value is recorded, and
//! market value / unrealized P&L are computed only once a price has been set.

use async_trait::async_trait;

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::{BotModule, CommandContext, CommandHelp, ModuleContext};

pub mod alerts;
pub mod alerts_handler;
pub mod audit;
pub mod cash;
pub mod cash_repo;
pub mod handler;
pub mod history;
pub mod history_feed;
pub mod models;
pub mod price_feed;
pub mod repository;
pub mod scheduler;
pub mod trade;
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
        db: &Database,
        broadcaster: Option<&EventBroadcaster>,
    ) -> Option<String> {
        handler::handle_portfolio_command(&db.pool, &ctx.owner_id, &ctx.full_body, broadcaster)
            .await
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

    /// Buying now requires cash on hand; this deposits comfortably more than
    /// any single test spends, via the command path itself (not the repository
    /// directly), so it also exercises `!portfolio cash deposit`.
    async fn fund(module: &PortfolioModule, db: &Database, currency: &str) {
        module
            .handle_command(
                &ctx(&format!("!portfolio cash deposit 100000 {currency}")),
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

    /// The module must claim its own commands and nothing else, or it will
    /// swallow another module's traffic — dispatch returns the first match.
    #[tokio::test]
    async fn test_claims_only_its_own_commands() {
        let module = PortfolioModule::new();
        let db = db().await;
        fund(&module, &db, "USD").await;

        assert!(module
            .handle_command(&ctx("!portfolio"), &db, None)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!portfolio buy AAPL 10 @ 150.00"), &db, None)
            .await
            .is_some());
        assert!(module
            .handle_command(&ctx("!portfolio cash"), &db, None)
            .await
            .is_some());

        assert!(module
            .handle_command(&ctx("!todo add milk"), &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!expense add 1 EUR x"), &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("!settings"), &db, None)
            .await
            .is_none());
        assert!(module
            .handle_command(&ctx("hello there"), &db, None)
            .await
            .is_none());
    }

    /// An end-to-end pass: deposit cash, buy a position, then read it back.
    #[tokio::test]
    async fn test_buy_then_list() {
        let module = PortfolioModule::new();
        let db = db().await;
        fund(&module, &db, "USD").await;

        let reply = module
            .handle_command(&ctx("!portfolio buy AAPL 10 @ 150.00 USD"), &db, None)
            .await
            .expect("the module must answer");
        assert!(reply.contains("AAPL"));
        assert!(
            reply.contains("Cash remaining: 98500.00 USD"),
            "should state the cash left: {reply}"
        );

        let listed = module
            .handle_command(&ctx("!portfolio list"), &db, None)
            .await
            .unwrap();
        assert!(
            listed.contains("AAPL"),
            "the position should be listed: {listed}"
        );
    }

    /// Buying without enough cash must be rejected, not silently succeed —
    /// this is the whole point of tracking cash at all.
    #[tokio::test]
    async fn test_buy_without_enough_cash_is_rejected() {
        let module = PortfolioModule::new();
        let db = db().await;

        let reply = module
            .handle_command(&ctx("!portfolio buy AAPL 10 @ 150.00 USD"), &db, None)
            .await
            .unwrap();
        assert!(
            reply.contains("Need"),
            "should explain the shortfall: {reply}"
        );

        // The empty-state message's own usage example mentions "AAPL", so
        // assert the positive (still empty), not a naive "does not contain".
        let listed = module
            .handle_command(&ctx("!portfolio list"), &db, None)
            .await
            .unwrap();
        assert!(
            listed.contains("No holdings yet"),
            "a rejected buy must not create a position: {listed}"
        );
    }

    /// The currency default comes from the sender's saved settings, exactly
    /// like the finance module's `!expense add`.
    #[tokio::test]
    async fn test_buy_without_a_currency_uses_the_senders_default() {
        use crate::modules::settings::{SettingsRepository, UserSettings};

        let module = PortfolioModule::new();
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
        fund(&module, &db, "CNY").await;

        let reply = module
            .handle_command(&ctx("!portfolio buy AAPL 10 @ 150.00"), &db, None)
            .await
            .unwrap();
        assert!(
            reply.contains("CNY"),
            "should use the saved currency: {reply}"
        );
    }

    /// Mutations must reach the SSE stream, exactly like the settings module —
    /// otherwise an open web tab never learns a Matrix-side buy happened. A
    /// buy touches two tables, so it must announce both.
    #[tokio::test]
    async fn test_buy_broadcasts_holding_and_cash_to_the_owner() {
        let module = PortfolioModule::new();
        let db = db().await;
        let broadcaster = EventBroadcaster::default();
        fund(&module, &db, "USD").await;
        let mut rx = broadcaster.subscribe();

        module
            .handle_command(
                &ctx("!portfolio buy AAPL 10 @ 150.00 USD"),
                &db,
                Some(&broadcaster),
            )
            .await
            .unwrap();

        let mut saw_holding = false;
        let mut saw_cash = false;
        while let Ok(event) = rx.try_recv() {
            assert_eq!(event.owner_id(), "@alice:a");
            match event {
                crate::events::SystemEvent::PortfolioHoldingCreated(_) => saw_holding = true,
                crate::events::SystemEvent::PortfolioCashUpdated { balance_minor, .. } => {
                    saw_cash = true;
                    // 100,000.00 deposited, less the 1,500.00 the buy cost.
                    assert_eq!(balance_minor, 10_000_000 - 150_000);
                }
                _ => {}
            }
        }
        assert!(saw_holding, "a buy must broadcast the holding");
        assert!(saw_cash, "a buy must broadcast the debited cash");
    }

    /// A read must not broadcast — nothing changed.
    #[tokio::test]
    async fn test_list_does_not_broadcast() {
        let module = PortfolioModule::new();
        let db = db().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        module
            .handle_command(&ctx("!portfolio list"), &db, Some(&broadcaster))
            .await
            .unwrap();
        module
            .handle_command(&ctx("!portfolio cash"), &db, Some(&broadcaster))
            .await
            .unwrap();

        assert!(rx.try_recv().is_err(), "a read must not broadcast");
    }

    /// A note is user-controlled text that reaches an HTML message both in the
    /// command's own confirmation and later in `!portfolio list`, so both must
    /// escape it.
    #[tokio::test]
    async fn test_escapes_user_text_in_replies() {
        let module = PortfolioModule::new();
        let db = db().await;
        fund(&module, &db, "USD").await;

        module
            .handle_command(&ctx("!portfolio buy AAPL 10 @ 150.00 USD"), &db, None)
            .await
            .unwrap();

        let note_reply = module
            .handle_command(
                &ctx("!portfolio note AAPL <script>alert(1)</script>"),
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
            .handle_command(&ctx("!portfolio list"), &db, None)
            .await
            .unwrap();
        assert!(
            !listed.contains("<script>"),
            "raw markup must not survive: {listed}"
        );
        assert!(listed.contains("&lt;script&gt;"));
    }

    /// End-to-end through the cash sub-commands: deposit, buy against it, sell
    /// back, and withdraw — the balance must reflect every step.
    #[tokio::test]
    async fn test_cash_deposit_buy_sell_withdraw_round_trip() {
        let module = PortfolioModule::new();
        let db = db().await;

        let deposited = module
            .handle_command(&ctx("!portfolio cash deposit 1000 USD"), &db, None)
            .await
            .unwrap();
        assert!(deposited.contains("1000.00"), "{deposited}");

        module
            .handle_command(&ctx("!portfolio buy AAPL 5 @ 150.00 USD"), &db, None)
            .await
            .unwrap();
        // 1000.00 - 750.00 (5 @ 150.00) = 250.00 remaining.
        let after_buy = module
            .handle_command(&ctx("!portfolio cash"), &db, None)
            .await
            .unwrap();
        assert!(after_buy.contains("250.00"), "{after_buy}");

        module
            .handle_command(&ctx("!portfolio sell AAPL 5 @ 180.00"), &db, None)
            .await
            .unwrap();
        // 250.00 + 900.00 (5 @ 180.00 proceeds) = 1150.00.
        let after_sell = module
            .handle_command(&ctx("!portfolio cash"), &db, None)
            .await
            .unwrap();
        assert!(after_sell.contains("1150.00"), "{after_sell}");

        let withdrawn = module
            .handle_command(&ctx("!portfolio cash withdraw 1150 USD"), &db, None)
            .await
            .unwrap();
        assert!(withdrawn.contains("0.00"), "{withdrawn}");

        // Verify deposit and withdraw transactions were posted to the finance ledger
        let txs = crate::modules::finance::repository::FinanceRepository::list(
            &db.pool, "@alice:a", None, 10, 0,
        )
        .await
        .unwrap();
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].direction, "income");
        assert_eq!(txs[0].amount_minor, 115000);
        assert_eq!(txs[0].source_kind.as_deref(), Some("portfolio_cash"));
        assert_eq!(txs[1].direction, "expense");
        assert_eq!(txs[1].amount_minor, 100000);
        assert_eq!(txs[1].source_kind.as_deref(), Some("portfolio_cash"));
    }

    /// Withdrawing more than is on hand must be rejected and say why.
    #[tokio::test]
    async fn test_cash_withdraw_insufficient_funds() {
        let module = PortfolioModule::new();
        let db = db().await;
        module
            .handle_command(&ctx("!portfolio cash deposit 100 USD"), &db, None)
            .await
            .unwrap();

        let reply = module
            .handle_command(&ctx("!portfolio cash withdraw 101 USD"), &db, None)
            .await
            .unwrap();
        assert!(
            reply.contains("100.00"),
            "should state what is actually available: {reply}"
        );
    }
}
