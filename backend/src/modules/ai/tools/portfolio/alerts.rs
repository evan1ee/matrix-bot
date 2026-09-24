use crate::db::DbPool;
use serde_json::json;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::infrastructure::inference::llm::ToolDefinition;
use crate::modules::finance::money::{exponent_for, parse_amount};
use crate::modules::portfolio::alerts::{AlertCondition, AlertsRepository};
use crate::modules::portfolio::models::normalise_symbol;
use crate::modules::portfolio::repository::PortfolioRepository;
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_tool(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
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

                let default_cur = SettingsRepository::get_or_default(pool, owner_id)
                    .await
                    .currency;
                let holding = PortfolioRepository::find(pool, owner_id, &symbol)
                    .await
                    .map_err(|e| e.to_string())?;
                let currency = holding
                    .as_ref()
                    .map(|h| h.currency.clone())
                    .unwrap_or(default_cur);
                let exp = i64::from(exponent_for(&currency));
                let target_price_minor =
                    parse_amount(&price_str, exp as u32).map_err(|e| e.to_string())?;

                let alert = AlertsRepository::create(
                    pool,
                    owner_id,
                    &symbol,
                    target_price_minor,
                    &currency,
                    exp,
                    condition,
                )
                .await
                .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::PortfolioAlertCreated(alert.clone()));
                }
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
                let alerts = AlertsRepository::list_for_owner(pool, owner_id)
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
                if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                    let deleted = AlertsRepository::delete(pool, id, owner_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    if deleted {
                        if let Some(b) = broadcaster {
                            b.send(SystemEvent::PortfolioAlertDeleted {
                                id,
                                owner_id: owner_id.to_string(),
                            });
                        }
                        Ok(json!({ "cancelled": true, "id": id }).to_string())
                    } else {
                        Err(format!("Alert #{} not found or not owned by user", id))
                    }
                } else if let Some(sym_str) = args.get("symbol").and_then(|v| v.as_str()) {
                    let symbol = normalise_symbol(sym_str)?;
                    let alerts = AlertsRepository::list_for_owner(pool, owner_id)
                        .await
                        .map_err(|e| e.to_string())?;
                    let to_cancel: Vec<_> = alerts
                        .into_iter()
                        .filter(|a| a.symbol == symbol && !a.is_triggered)
                        .collect();
                    let count = to_cancel.len();
                    for a in to_cancel {
                        let _ = AlertsRepository::delete(pool, a.id, owner_id).await;
                        if let Some(b) = broadcaster {
                            b.send(SystemEvent::PortfolioAlertDeleted {
                                id: a.id,
                                owner_id: owner_id.to_string(),
                            });
                        }
                    }
                    Ok(json!({ "cancelled": true, "symbol": symbol, "count": count }).to_string())
                } else {
                    Err("Either 'id' or 'symbol' must be provided to cancel an alert".to_string())
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
            "Set a price alert trigger on an asset when it crosses above or below a target price.",
            json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Ticker symbol (required, e.g. AAPL, BTC)"
                    },
                    "target_price": {
                        "type": "number",
                        "description": "Target price trigger level (required)"
                    },
                    "condition": {
                        "type": "string",
                        "enum": ["above", "below"],
                        "description": "Trigger condition: 'above' or 'below' (required)"
                    },
                    "currency": {
                        "type": "string",
                        "description": "3-letter currency code"
                    }
                },
                "required": ["symbol", "target_price", "condition"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_portfolio_price_alerts",
            "List active price alerts configured for the user's portfolio.",
            json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Optional ticker symbol filter"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "cancel_portfolio_price_alert",
            "Cancel or delete a price alert by numeric ID or all alerts for a given ticker symbol.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "Alert ID to cancel"
                    },
                    "symbol": {
                        "type": "string",
                        "description": "Cancel all alerts for this ticker symbol"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
    ]
}
