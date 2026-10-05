use crate::db::DbPool;
use crate::domain::accounts::AccountId;
pub mod bills;
pub mod bookmarks;
pub mod definitions;
pub mod executor;
pub mod finance;
pub mod habits;
pub mod knowledge;
pub mod life;
pub mod portfolio;
pub mod reminder;
pub mod settings;
pub mod todo;
pub mod webhooks;

use crate::infrastructure::inference::llm::ToolDefinition;

pub struct ToolRegistry;

impl ToolRegistry {
    pub fn get_tool_definitions() -> Vec<ToolDefinition> {
        definitions::all_tool_definitions()
    }

    pub async fn execute_tool(
        services: &crate::application::AppServices,
        pool: &DbPool,
        owner_id: &AccountId,
        room_id: &str,
        name: &str,
        args_json: &str,
    ) -> Result<String, String> {
        executor::execute(services, pool, owner_id, room_id, name, args_json).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::infrastructure::postgres::repositories::finance as finance_repo;

    async fn setup_db() -> (Database, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let db = Database { pool };
        let account_id = crate::application::AppServices::new(&db.pool)
            .identity
            .provision_matrix_for_test("@alice:mbm.one")
            .await;
        (db, account_id)
    }

    #[tokio::test]
    async fn test_tool_definitions() {
        let defs = ToolRegistry::get_tool_definitions();
        assert_eq!(defs.len(), 51);
        let names: Vec<_> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"get_my_tasks"));
        assert!(names.contains(&"add_todo"));
        assert!(names.contains(&"complete_todo"));
        assert!(names.contains(&"update_todo"));
        assert!(names.contains(&"delete_todo"));
        assert!(names.contains(&"record_finance_entry"));
        assert!(names.contains(&"list_finance_transactions"));
        assert!(names.contains(&"get_spending_by_category"));
        assert!(names.contains(&"delete_finance_entry"));
        assert!(names.contains(&"set_reminder"));
        assert!(names.contains(&"cancel_reminder"));
        assert!(names.contains(&"deposit_portfolio_cash"));
        assert!(names.contains(&"withdraw_portfolio_cash"));
        assert!(names.contains(&"buy_portfolio_holding"));
        assert!(names.contains(&"sell_portfolio_holding"));
        assert!(names.contains(&"get_portfolio_holding"));
        assert!(names.contains(&"create_portfolio_price_alert"));
        assert!(names.contains(&"get_portfolio_price_alerts"));
        assert!(names.contains(&"cancel_portfolio_price_alert"));
        assert!(names.contains(&"get_portfolio_performance_history"));
        assert!(names.contains(&"get_my_habits"));
        assert!(names.contains(&"log_habit"));
        assert!(names.contains(&"create_habit"));
        assert!(names.contains(&"delete_habit"));
        assert!(names.contains(&"log_life_day"));
        assert!(names.contains(&"get_life_profile"));
        assert!(names.contains(&"get_daily_log"));
        assert!(names.contains(&"get_my_recurring_bills"));
        assert!(names.contains(&"pay_recurring_bill"));
        assert!(names.contains(&"create_recurring_bill"));
        assert!(names.contains(&"delete_recurring_bill"));
        assert!(names.contains(&"get_my_footprints"));
        assert!(names.contains(&"record_footprint"));
        assert!(names.contains(&"delete_footprint"));
        assert!(names.contains(&"get_weekly_retrospective"));
        assert!(names.contains(&"save_bookmark"));
        assert!(names.contains(&"get_my_bookmarks"));
        assert!(names.contains(&"toggle_bookmark_favorite"));
        assert!(names.contains(&"archive_bookmark"));
        assert!(names.contains(&"delete_bookmark"));
        assert!(names.contains(&"knowledge_search"));
        assert!(names.contains(&"knowledge_create_note"));
        assert!(names.contains(&"knowledge_list_tags"));
        assert!(names.contains(&"get_my_webhooks"));
        assert!(names.contains(&"get_webhook_logs"));
        assert!(names.contains(&"get_my_settings"));
        assert!(names.contains(&"update_my_settings"));
    }

    /// AGENTS.md section 5.3: the owner comes from the authenticated context,
    /// never from the model. Arguments naming another account are ignored.
    #[tokio::test]
    async fn a_tool_cannot_act_for_another_account_named_in_its_arguments() {
        let (db, a) = setup_db().await;
        let b = AccountId::from_seed("victim");
        crate::db::pool::seed_account(&db.pool, &b).await;

        for field in ["owner_id", "account_id"] {
            let args = serde_json::json!({ "text": "sneaky", field: b.to_string() }).to_string();
            ToolRegistry::execute_tool(
                &crate::application::AppServices::new(&db.pool),
                &db.pool,
                &a,
                "!room:a",
                "add_todo",
                &args,
            )
            .await
            .unwrap();
        }

        let list = |owner: AccountId| {
            let pool = db.pool.clone();
            async move {
                crate::infrastructure::postgres::repositories::todos::list_all_user(
                    &pool, &owner, None, 50, 0,
                )
                .await
                .unwrap()
            }
        };
        assert_eq!(list(a).await.len(), 2);
        assert!(list(b).await.is_empty());
    }

    #[tokio::test]
    async fn test_execute_add_and_complete_todo() {
        let (db, owner) = setup_db().await;

        let add_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "add_todo",
            r#"{"text":"Buy oat milk","priority":"high","due_in_minutes":60}"#,
        )
        .await
        .unwrap();
        assert!(add_res.contains("Buy oat milk"));

        let complete_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "complete_todo",
            r#"{"text_search":"oat milk"}"#,
        )
        .await
        .unwrap();
        assert!(complete_res.contains(r#""completed":true"#));
    }

    #[tokio::test]
    async fn test_execute_finance_and_reminder_tools() {
        let (db, owner) = setup_db().await;

        let fin_res = ToolRegistry::execute_tool( &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "record_finance_entry",
            r#"{"amount":"15.50","direction":"expense","note":"Lunch with team","category":"Food"}"#,
        )
        .await
        .unwrap();
        assert!(fin_res.contains("Lunch with team"));
        assert!(fin_res.contains("Food"));

        let rem_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "set_reminder",
            r#"{"message":"Check staging logs","in_minutes":30}"#,
        )
        .await
        .unwrap();
        assert!(rem_res.contains("Check staging logs"));
    }

    #[tokio::test]
    async fn test_execute_portfolio_deposit_and_buy() {
        let (db, owner) = setup_db().await;

        let dep_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "deposit_portfolio_cash",
            r#"{"amount":"2000.00","currency":"USD"}"#,
        )
        .await
        .unwrap();
        assert!(dep_res.contains("200000"));

        let buy_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "buy_portfolio_holding",
            r#"{"symbol":"AAPL","quantity":"5","price":"150.00","currency":"USD"}"#,
        )
        .await
        .unwrap();
        assert!(buy_res.contains("AAPL"));
    }

    #[tokio::test]
    async fn test_execute_portfolio_holding_sell_and_withdraw() {
        let (db, owner) = setup_db().await;

        // Deposit cash
        ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "deposit_portfolio_cash",
            r#"{"amount":"5000.00","currency":"USD"}"#,
        )
        .await
        .unwrap();

        // Buy 10 AAPL @ 150 = 1500
        ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "buy_portfolio_holding",
            r#"{"symbol":"AAPL","quantity":"10","price":"150.00","currency":"USD"}"#,
        )
        .await
        .unwrap();

        // Query holding details
        let holding_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_portfolio_holding",
            r#"{"symbol":"AAPL"}"#,
        )
        .await
        .unwrap();
        assert!(holding_res.contains("AAPL"));
        assert!(holding_res.contains("15000")); // avg_cost_minor

        // Sell 4 AAPL @ 180 = 720
        let sell_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "sell_portfolio_holding",
            r#"{"symbol":"AAPL","quantity":"4","price":"180.00"}"#,
        )
        .await
        .unwrap();
        assert!(sell_res.contains("reduced"));
        assert!(sell_res.contains("AAPL"));

        // Withdraw 500 cash
        let withdraw_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "withdraw_portfolio_cash",
            r#"{"amount":"500.00","currency":"USD"}"#,
        )
        .await
        .unwrap();
        assert!(withdraw_res.contains("balance_minor"));
    }

    #[tokio::test]
    async fn test_execute_portfolio_price_alerts_and_history() {
        let (db, owner) = setup_db().await;

        // Create price alert
        let alert_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "create_portfolio_price_alert",
            r#"{"symbol":"BTC-USD","condition":"gte","target_price":"70000.00"}"#,
        )
        .await
        .unwrap();
        assert!(alert_res.contains("BTC-USD"));
        assert!(alert_res.contains("7000000"));

        // List alerts
        let list_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_portfolio_price_alerts",
            r#"{"active_only":true}"#,
        )
        .await
        .unwrap();
        assert!(list_res.contains("BTC-USD"));

        // Cancel alert by symbol
        let cancel_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "cancel_portfolio_price_alert",
            r#"{"symbol":"BTC-USD"}"#,
        )
        .await
        .unwrap();
        assert!(cancel_res.contains(r#""cancelled":true"#));

        // Performance history
        let history_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_portfolio_performance_history",
            r#"{"timeframe":"1m","currency":"USD"}"#,
        )
        .await
        .unwrap();
        assert!(history_res.contains("period_change_pct"));
    }

    #[tokio::test]
    async fn test_execute_finance_transactions_and_category_spending() {
        let (db, owner) = setup_db().await;

        // Record two entries
        let e1 = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "record_finance_entry",
            r#"{"amount":"45.00","direction":"expense","note":"Groceries","category":"Food"}"#,
        )
        .await
        .unwrap();
        assert!(e1.contains("Groceries"));

        ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "record_finance_entry",
            r#"{"amount":"20.00","direction":"expense","note":"Metro pass","category":"Transit"}"#,
        )
        .await
        .unwrap();

        // List transactions
        let list_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "list_finance_transactions",
            r#"{"limit":5}"#,
        )
        .await
        .unwrap();
        assert!(list_res.contains("Groceries"));
        assert!(list_res.contains("Metro pass"));

        // Category spending
        let spending_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_spending_by_category",
            r#"{"days":30}"#,
        )
        .await
        .unwrap();
        assert!(spending_res.contains("Food"));
        assert!(spending_res.contains("Transit"));
    }

    #[tokio::test]
    async fn test_record_finance_entry_with_llm_json_schema() {
        let (db, owner) = setup_db().await;

        // LLM calls record_finance_entry with number amount, entry_type, description, occurred_at
        let res = ToolRegistry::execute_tool( &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "record_finance_entry",
            r#"{"category":"Food","description":"纽仕兰4.0全脂纯牛奶 250ml*24盒","entry_type":"expense","amount":87.70,"currency":"CNY","occurred_at":"2026-09-09"}"#,
        )
        .await
        .unwrap();

        assert!(res.contains("纽仕兰4.0全脂纯牛奶"));
        assert!(res.contains("8770"));
        assert!(res.contains("CNY"));

        // Verify transaction was persisted
        let txs = finance_repo::list(
            &db.pool,
            &owner,
            &crate::domain::finance::ListTransactionsFilter {
                limit: Some(10),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].amount_minor, 8770);
        assert_eq!(txs[0].currency, "CNY");
        assert_eq!(
            txs[0].note.as_deref(),
            Some("纽仕兰4.0全脂纯牛奶 250ml*24盒")
        );
    }

    #[tokio::test]
    async fn test_execute_life_and_habits_and_retro_tools() {
        let (db, owner) = setup_db().await;

        // 1. Create habit
        let create_habit_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "create_habit",
            r#"{"name":"Morning Run","frequency":"daily"}"#,
        )
        .await
        .unwrap();
        assert!(create_habit_res.contains("Morning Run"));

        // 2. Log habit
        let log_habit_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "log_habit",
            r#"{"name":"Morning Run","completed":true}"#,
        )
        .await
        .unwrap();
        assert!(log_habit_res.contains("Morning Run"));

        // 3. Log life day
        let log_life_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "log_life_day",
            r#"{"mood":"amazing","activities":["Running","Coding"],"note":"Great day!"}"#,
        )
        .await
        .unwrap();
        assert!(log_life_res.contains("amazing"));

        // 4. Record footprint
        let footprint_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "record_footprint",
            r#"{"title":"Coffee Shop","category":"cafe","latitude":37.7749,"longitude":-122.4194}"#,
        )
        .await
        .unwrap();
        assert!(footprint_res.contains("Coffee Shop"));

        // 5. Weekly retrospective
        let retro_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_weekly_retrospective",
            r#"{}"#,
        )
        .await
        .unwrap();
        assert!(retro_res.contains("Weekly Executive Retrospective"));

        // 6. Bookmarks
        let save_res = ToolRegistry::execute_tool( &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "save_bookmark",
            r#"{"url":"https://example.com/rust-guide","title":"Rust Guide","tags":["rust","programming"]}"#,
        )
        .await
        .unwrap();
        assert!(save_res.contains("Rust Guide"));
        assert!(save_res.contains("example.com"));

        let list_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_my_bookmarks",
            r#"{"tag":"rust"}"#,
        )
        .await
        .unwrap();
        assert!(list_res.contains("Rust Guide"));
    }

    #[tokio::test]
    async fn test_execute_extended_tools() {
        let (db, owner) = setup_db().await;

        // 1. Todo update & delete
        let add_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "add_todo",
            r#"{"text":"Task to update","priority":"low"}"#,
        )
        .await
        .unwrap();
        let todo_json: serde_json::Value = serde_json::from_str(&add_res).unwrap();
        let todo_id = todo_json["id"].as_i64().unwrap();

        let update_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "update_todo",
            &format!(
                r#"{{"id":{},"text":"Task updated","priority":"high"}}"#,
                todo_id
            ),
        )
        .await
        .unwrap();
        assert!(update_res.contains("Task updated"));
        assert!(update_res.contains("high"));

        let delete_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "delete_todo",
            &format!(r#"{{"id":{}}}"#, todo_id),
        )
        .await
        .unwrap();
        assert!(delete_res.contains(r#""deleted":true"#));

        // 2. Reminder cancel
        let rem_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "set_reminder",
            r#"{"message":"Doctor appointment","in_minutes":120}"#,
        )
        .await
        .unwrap();
        let rem_json: serde_json::Value = serde_json::from_str(&rem_res).unwrap();
        let rem_id = rem_json["id"].as_i64().unwrap();

        let cancel_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "cancel_reminder",
            &format!(r#"{{"id":{}}}"#, rem_id),
        )
        .await
        .unwrap();
        assert!(cancel_res.contains(r#""cancelled":true"#));

        // 3. Recurring bill create & delete
        let bill_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "create_recurring_bill",
            r#"{"name":"Netflix","amount":"15.99","currency":"USD","category":"subscriptions"}"#,
        )
        .await
        .unwrap();
        assert!(bill_res.contains("Netflix"));
        let bill_json: serde_json::Value = serde_json::from_str(&bill_res).unwrap();
        let bill_id = bill_json["id"].as_i64().unwrap();

        let del_bill_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "delete_recurring_bill",
            &format!(r#"{{"id":{}}}"#, bill_id),
        )
        .await
        .unwrap();
        assert!(del_bill_res.contains(r#""deleted":true"#));

        // 4. Habit delete
        let habit_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "create_habit",
            r#"{"name":"Daily Pushups","category":"fitness"}"#,
        )
        .await
        .unwrap();
        let habit_json: serde_json::Value = serde_json::from_str(&habit_res).unwrap();
        let habit_id = habit_json["id"].as_i64().unwrap();

        let del_habit_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "delete_habit",
            &format!(r#"{{"id":{}}}"#, habit_id),
        )
        .await
        .unwrap();
        assert!(del_habit_res.contains(r#""deleted":true"#));

        // 5. Daily log query
        let daily_log_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_daily_log",
            r#"{}"#,
        )
        .await
        .unwrap();
        assert!(daily_log_res.contains("date"));

        // 6. Settings update
        let settings_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "update_my_settings",
            r#"{"timezone":"America/New_York","currency":"EUR"}"#,
        )
        .await
        .unwrap();
        assert!(settings_res.contains("America/New_York"));
        assert!(settings_res.contains("EUR"));

        // 7. Webhooks list
        let webhooks_res = ToolRegistry::execute_tool(
            &crate::application::AppServices::new(&db.pool),
            &db.pool,
            &owner,
            "!room:a",
            "get_my_webhooks",
            r#"{}"#,
        )
        .await
        .unwrap();
        assert!(webhooks_res.starts_with('['));
    }
}
