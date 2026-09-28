use crate::db::DbPool;
use serde_json::json;

use crate::domain::finance::money::{exponent_for, parse_amount};
use crate::domain::portfolio::alerts::AlertCondition;
use crate::domain::portfolio::models::normalise_symbol;
use crate::events::EventBroadcaster;
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    _pool: &DbPool,
    _broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "create_portfolio_price_alert" => Some(
            async {
                let raw_symbol = args
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'symbol' is required")?;
                let symbol = normalise_symbol(raw_symbol)?;

                let cond_str = args
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .unwrap_or("above");
                let condition = match cond_str.to_lowercase().as_str() {
                    "gte" | "above" | ">" | ">=" | "higher" => AlertCondition::Above,
                    "lte" | "below" | "<" | "<=" | "lower" => AlertCondition::Below,
                    _ => return Err("Invalid condition: must be 'above' or 'below'".to_string()),
                };

                let price_str = if let Some(num) = args.get("target_price").and_then(|v| v.as_f64())
                {
                    format!("{:.2}", num)
                } else if let Some(s) = args.get("target_price").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'target_price' is required".to_string());
                };

                let acct = crate::domain::accounts::AccountId::from(owner_id);
                let default_cur = services.settings.get_or_default(&acct).await.currency;
                let holding = services
                    .portfolio
                    .get_holding_by_symbol(&acct, &symbol)
                    .await
                    .map_err(|e| e.to_string())?;
                let currency = holding
                    .as_ref()
                    .map(|h| h.currency.clone())
                    .unwrap_or(default_cur);
                let exp = i64::from(exponent_for(&currency));
                let target_price_minor =
                    parse_amount(&price_str, exp as u32).map_err(|e| e.to_string())?;

                let alert = services
                    .portfolio
                    .create_alert(
                        &acct,
                        &symbol,
                        target_price_minor,
                        &currency,
                        exp,
                        condition,
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&alert).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_portfolio_price_alerts" => Some(
            async {
                let active_only = args
                    .get("active_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let acct = crate::domain::accounts::AccountId::from(owner_id);
                let alerts = services
                    .portfolio
                    .list_alerts(&acct)
                    .await
                    .map_err(|e| e.to_string())?;

                let filtered: Vec<_> = if active_only {
                    alerts.into_iter().filter(|a| !a.is_triggered).collect()
                } else {
                    alerts
                };
                serde_json::to_string(&filtered).map_err(|e| e.to_string())
            }
            .await,
        ),

        "cancel_portfolio_price_alert" => Some(
            async {
                let id_opt = args
                    .get("id")
                    .or_else(|| args.get("alert_id"))
                    .and_then(|v| v.as_i64());

                if let Some(id) = id_opt {
                    let acct = crate::domain::accounts::AccountId::from(owner_id);
                    let deleted = services
                        .portfolio
                        .delete_alert(&acct, id)
                        .await
                        .map_err(|e| e.to_string())?;

                    if deleted {
                        Ok(json!({
                            "cancelled": true,
                            "id": id,
                            "message": format!("Price alert #{} successfully cancelled", id)
                        })
                        .to_string())
                    } else {
                        Err(format!("Alert #{} not found or access denied", id))
                    }
                } else if let Some(sym_str) = args.get("symbol").and_then(|v| v.as_str()) {
                    let symbol = normalise_symbol(sym_str)?;
                    let acct = crate::domain::accounts::AccountId::from(owner_id);
                    let alerts = services
                        .portfolio
                        .list_alerts(&acct)
                        .await
                        .map_err(|e| e.to_string())?;
                    let to_cancel: Vec<_> = alerts
                        .into_iter()
                        .filter(|a| a.symbol == symbol && !a.is_triggered)
                        .collect();
                    let count = to_cancel.len();
                    for a in to_cancel {
                        let _ = services.portfolio.delete_alert(&acct, a.id).await;
                    }
                    Ok(json!({
                        "cancelled": true,
                        "symbol": symbol,
                        "count": count
                    })
                    .to_string())
                } else {
                    Err("Either 'id'/'alert_id' or 'symbol' must be provided to cancel an alert".to_string())
                }
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "create_portfolio_price_alert",
            "Set a target price alert on an investment symbol or cryptocurrency (e.g. notify when BTC goes above 70000).",
            json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Ticker symbol or pair, e.g. 'AAPL', 'BTC-USD'" },
                    "target_price": { "type": "string", "description": "Target trigger price as decimal string, e.g. '180.50' or '70000'" },
                    "condition": { "type": "string", "enum": ["above", "below"], "description": "Condition: 'above' or 'below'" }
                },
                "required": ["symbol", "target_price"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_portfolio_price_alerts",
            "List price alerts configured by the user, optionally filtering only active ones.",
            json!({
                "type": "object",
                "properties": {
                    "active_only": { "type": "boolean", "description": "If true, only returns active un-triggered alerts. Defaults to false." }
                },
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "cancel_portfolio_price_alert",
            "Cancel or delete a price alert by numeric ID or all alerts for a given ticker symbol.",
            json!({
                "type": "object",
                "properties": {
                    "id": { "type": "integer", "description": "Alert ID to cancel" },
                    "alert_id": { "type": "integer", "description": "Alert ID to cancel (alias for id)" },
                    "symbol": { "type": "string", "description": "Cancel all alerts for this ticker symbol" }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
    ]
}
