pub mod definitions;
pub mod executor;

use sqlx::SqlitePool;
use crate::events::EventBroadcaster;
use crate::modules::ai::models::ToolDefinition;

pub struct ToolRegistry;

impl ToolRegistry {
    pub fn get_tool_definitions() -> Vec<ToolDefinition> {
        definitions::all_tool_definitions()
    }

    pub async fn execute_tool(
        pool: &SqlitePool,
        broadcaster: Option<&EventBroadcaster>,
        owner_id: &str,
        room_id: &str,
        name: &str,
        args_json: &str,
    ) -> Result<String, String> {
        executor::execute(pool, broadcaster, owner_id, room_id, name, args_json).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    async fn setup_db() -> (Database, String) {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();
        crate::db::schema::run_migrations(&db.pool).await.unwrap();
        let account_id = crate::accounts::AccountRepository::resolve_or_provision(&db.pool, "@alice:mbm.one")
            .await
            .unwrap();
        (db, account_id)
    }

    #[tokio::test]
    async fn test_tool_definitions() {
        let defs = ToolRegistry::get_tool_definitions();
        assert_eq!(defs.len(), 12);
        let names: Vec<_> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"get_my_tasks"));
        assert!(names.contains(&"add_todo"));
        assert!(names.contains(&"complete_todo"));
        assert!(names.contains(&"record_finance_entry"));
        assert!(names.contains(&"set_reminder"));
        assert!(names.contains(&"deposit_portfolio_cash"));
        assert!(names.contains(&"buy_portfolio_holding"));
    }

    #[tokio::test]
    async fn test_execute_add_and_complete_todo() {
        let (db, owner) = setup_db().await;

        let add_res = ToolRegistry::execute_tool(
            &db.pool,
            None,
            &owner,
            "!room:a",
            "add_todo",
            r#"{"text":"Buy oat milk","priority":"high","due_in_minutes":60}"#,
        )
        .await
        .unwrap();
        assert!(add_res.contains("Buy oat milk"));

        let complete_res = ToolRegistry::execute_tool(
            &db.pool,
            None,
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

        let fin_res = ToolRegistry::execute_tool(
            &db.pool,
            None,
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
            &db.pool,
            None,
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
            &db.pool,
            None,
            &owner,
            "!room:a",
            "deposit_portfolio_cash",
            r#"{"amount":"2000.00","currency":"USD"}"#,
        )
        .await
        .unwrap();
        assert!(dep_res.contains("200000"));

        let buy_res = ToolRegistry::execute_tool(
            &db.pool,
            None,
            &owner,
            "!room:a",
            "buy_portfolio_holding",
            r#"{"symbol":"AAPL","quantity":"5","price":"150.00","currency":"USD"}"#,
        )
        .await
        .unwrap();
        assert!(buy_res.contains("AAPL"));
    }
}
