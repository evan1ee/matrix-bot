use std::collections::BTreeMap;
use serde_json::json;

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::portfolio::rebalance::evaluate_portfolio_drift;
use crate::infrastructure::inference::llm::ToolDefinition;

pub fn tool_definition() -> ToolDefinition {
    ToolDefinition::function(
        "evaluate_portfolio_rebalance",
        "Evaluate portfolio asset allocation drift against target weights and calculate rebalance actions (buy/sell amounts) without executing trades.",
        json!({
            "type": "object",
            "properties": {
                "targets": {
                    "type": "object",
                    "description": "Mapping of ticker symbols to target percentage weights (e.g. {\"AAPL\": 50.0, \"BTC\": 50.0}).",
                    "additionalProperties": { "type": "number" }
                },
                "threshold_pct": {
                    "type": "number",
                    "description": "Drift threshold percentage to trigger a rebalance action (default: 5.0)."
                }
            },
            "additionalProperties": false
        }),
    )
}

pub async fn execute_tool(
    services: &AppServices,
    owner_id: &AccountId,
    args: &serde_json::Value,
) -> Result<String, String> {
    let mut targets = BTreeMap::new();
    if let Some(obj) = args.get("targets").and_then(|v| v.as_object()) {
        for (k, v) in obj {
            if let Some(num) = v.as_f64() {
                targets.insert(k.trim().to_uppercase(), num);
            }
        }
    }

    let threshold_pct = args
        .get("threshold_pct")
        .and_then(|v| v.as_f64())
        .unwrap_or(5.0);

    let holdings = services
        .portfolio
        .list_holdings(owner_id)
        .await
        .map_err(|e| e.to_string())?;

    let report = evaluate_portfolio_drift(&holdings, &targets, threshold_pct);
    serde_json::to_string(&report).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPool;
    use crate::domain::portfolio::rebalance::PortfolioDriftReport;
    use crate::domain::portfolio::TradeRequest;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[test]
    fn test_rebalance_tool_definition() {
        let def = tool_definition();
        assert_eq!(def.function.name, "evaluate_portfolio_rebalance");
        let props = def.function.parameters.get("properties").unwrap();
        assert!(props.get("targets").is_some());
        assert!(props.get("threshold_pct").is_some());
    }

    #[tokio::test]
    async fn test_execute_portfolio_rebalance_e2e() {
        let pool = setup().await;
        let alice = AccountId::from_seed("@alice_rebalance:test");
        crate::db::pool::seed_account(&pool, &alice).await;
        let services = AppServices::new(&pool);

        // Deposit cash and buy holding
        services
            .portfolio
            .deposit_cash(&alice, "USD", 2, 500000, None)
            .await
            .unwrap();

        services
            .portfolio
            .buy(
                &alice,
                TradeRequest {
                    symbol: "AAPL",
                    quantity: "10",
                    price: "150.00",
                    currency: "USD",
                    note: None,
                    executed_at: None,
                },
            )
            .await
            .unwrap();

        services
            .portfolio
            .set_price(&alice, "AAPL", 15000)
            .await
            .unwrap();

        let args = json!({
            "targets": {
                "AAPL": 50.0,
                "GOOG": 50.0
            },
            "threshold_pct": 5.0
        });

        let result_json = execute_tool(&services, &alice, &args)
            .await
            .expect("tool execution should succeed");

        let report: PortfolioDriftReport =
            serde_json::from_str(&result_json).expect("valid JSON report");

        assert_eq!(report.total_value_minor, 150000);
        assert_eq!(report.allocations.len(), 2);
        assert!(report.needs_rebalance);
    }
}
