pub mod alerts;
pub mod cash;

use serde_json::json;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::ai::models::ToolDefinition;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::portfolio::history::HistoryRepository;
use crate::modules::portfolio::models::{cost_of, normalise_symbol, QUANTITY_EXPONENT};
use crate::modules::portfolio::repository::{BuyOutcome, PortfolioRepository, SellOutcome};
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    // 1. Check cash operations
    if let Some(res) = cash::execute_tool(pool, broadcaster, owner_id, name, args).await {
        return Some(res);
    }

    // 2. Check alert operations
    if let Some(res) = alerts::execute_tool(pool, broadcaster, owner_id, name, args).await {
        return Some(res);
    }

    // 3. Trade and portfolio holding operations
    match name {
        "get_my_portfolio_summary" => Some(
            async {
                let summary = PortfolioRepository::summary(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&summary).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_portfolio_holding" => Some(
            async {
                let raw_symbol = args
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'symbol' is required")?;
                let symbol = normalise_symbol(raw_symbol)?;
                let holding = PortfolioRepository::find(pool, owner_id, &symbol)
                    .await
                    .map_err(|e| e.to_string())?;

                match holding {
                    Some(h) => {
                        let mut obj = json!({
                            "id": h.id,
                            "symbol": h.symbol,
                            "quantity_minor": h.quantity_minor,
                            "avg_cost_minor": h.avg_cost_minor,
                            "currency": h.currency,
                            "currency_exponent": h.currency_exponent,
                            "current_price_minor": h.current_price_minor,
                            "price_updated_at": h.price_updated_at,
                            "note": h.note,
                        });

                        if let Some(price) = h.current_price_minor {
                            let current_val = cost_of(h.quantity_minor, price);
                            let cost_basis = cost_of(h.quantity_minor, h.avg_cost_minor);
                            let unrealized_pl = current_val - cost_basis;
                            let pl_pct = if cost_basis > 0 {
                                (unrealized_pl as f64 / cost_basis as f64) * 100.0
                            } else {
                                0.0
                            };
                            obj["current_value_minor"] = json!(current_val);
                            obj["unrealized_pl_minor"] = json!(unrealized_pl);
                            obj["unrealized_pl_pct"] = json!(pl_pct);
                        }
                        Ok(obj.to_string())
                    }
                    None => Ok(json!({
                        "found": false,
                        "symbol": symbol,
                        "message": format!("No open position found for symbol '{}'", symbol)
                    })
                    .to_string()),
                }
            }
            .await,
        ),

        "buy_portfolio_holding" => Some(
            async {
                let raw_symbol = args
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'symbol' is required")?;
                let symbol = normalise_symbol(raw_symbol)?;

                let qty_str = if let Some(num) = args.get("quantity").and_then(|v| v.as_f64()) {
                    num.to_string()
                } else if let Some(s) = args.get("quantity").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'quantity' is required".to_string());
                };

                let price_str = if let Some(num) = args.get("price").and_then(|v| v.as_f64()) {
                    format!("{:.2}", num)
                } else if let Some(s) = args.get("price").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'price' is required".to_string());
                };

                let note = args.get("note").and_then(|v| v.as_str());
                let default_cur = SettingsRepository::get_or_default(pool, owner_id)
                    .await
                    .currency;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur);
                let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
                let exp = exponent_for(&currency);

                let quantity_minor =
                    parse_amount(&qty_str, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;
                let price_minor = parse_amount(&price_str, exp).map_err(|e| e.to_string())?;

                let outcome = PortfolioRepository::buy(
                    pool,
                    owner_id,
                    &symbol,
                    quantity_minor,
                    price_minor,
                    &currency,
                    exp as i64,
                    note,
                )
                .await
                .map_err(|e| e.to_string())?;

                match outcome {
                    BuyOutcome::Ok { holding, cash } => {
                        if let Some(b) = broadcaster {
                            b.send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
                            b.send(SystemEvent::PortfolioCashUpdated {
                                owner_id: owner_id.to_string(),
                                currency: cash.currency.clone(),
                                balance_minor: cash.balance_minor,
                            });
                        }
                        serde_json::to_string(&json!({
                            "status": "bought",
                            "holding": holding,
                            "cash": cash,
                        }))
                        .map_err(|e| e.to_string())
                    }
                    BuyOutcome::InsufficientCash {
                        needed_minor,
                        available_minor,
                    } => Err(format!(
                        "Insufficient cash in {}: needed {} minor units, but only {} available",
                        currency, needed_minor, available_minor
                    )),
                    BuyOutcome::CurrencyMismatch { existing_currency } => Err(format!(
                        "Holding currency mismatch: existing holding is in {}",
                        existing_currency
                    )),
                }
            }
            .await,
        ),

        "sell_portfolio_holding" => Some(
            async {
                let raw_symbol = args
                    .get("symbol")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'symbol' is required")?;
                let symbol = normalise_symbol(raw_symbol)?;

                let qty_str = if let Some(num) = args.get("quantity").and_then(|v| v.as_f64()) {
                    num.to_string()
                } else if let Some(s) = args.get("quantity").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'quantity' is required".to_string());
                };

                let price_str = if let Some(num) = args.get("price").and_then(|v| v.as_f64()) {
                    format!("{:.2}", num)
                } else if let Some(s) = args.get("price").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'price' is required".to_string());
                };

                let existing = PortfolioRepository::find(pool, owner_id, &symbol)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("Position '{}' not found in portfolio", symbol))?;

                let exp = exponent_for(&existing.currency);
                let quantity_minor =
                    parse_amount(&qty_str, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;
                let price_minor = parse_amount(&price_str, exp).map_err(|e| e.to_string())?;

                let outcome = PortfolioRepository::sell(
                    pool,
                    owner_id,
                    &symbol,
                    quantity_minor,
                    price_minor,
                )
                .await
                .map_err(|e| e.to_string())?;

                match outcome {
                    SellOutcome::Closed {
                        realized_pl_minor,
                        cash,
                    } => {
                        if let Some(b) = broadcaster {
                            b.send(SystemEvent::PortfolioHoldingDeleted {
                                id: existing.id,
                                owner_id: owner_id.to_string(),
                            });
                            b.send(SystemEvent::PortfolioCashUpdated {
                                owner_id: owner_id.to_string(),
                                currency: cash.currency.clone(),
                                balance_minor: cash.balance_minor,
                            });
                        }
                        Ok(json!({
                            "status": "closed",
                            "symbol": symbol,
                            "realized_pl_minor": realized_pl_minor,
                            "cash": cash,
                            "message": format!("Fully closed {} position; proceeds credited to cash.", symbol)
                        }).to_string())
                    }
                    SellOutcome::Reduced {
                        holding,
                        realized_pl_minor,
                        cash,
                    } => {
                        if let Some(b) = broadcaster {
                            b.send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
                            b.send(SystemEvent::PortfolioCashUpdated {
                                owner_id: owner_id.to_string(),
                                currency: cash.currency.clone(),
                                balance_minor: cash.balance_minor,
                            });
                        }
                        Ok(json!({
                            "status": "reduced",
                            "holding": holding,
                            "realized_pl_minor": realized_pl_minor,
                            "cash": cash,
                            "message": format!("Partially sold {} position; proceeds credited to cash.", symbol)
                        }).to_string())
                    }
                    SellOutcome::NotFound => Err(format!("Position '{}' not found", symbol)),
                    SellOutcome::ExceedsHolding { held_minor } => Err(format!(
                        "Cannot sell more than held: you hold {} minor units, tried to sell {}",
                        held_minor, quantity_minor
                    )),
                }
            }
            .await,
        ),

        "get_portfolio_performance_history" => Some(
            async {
                let timeframe = args
                    .get("timeframe")
                    .and_then(|v| v.as_str())
                    .unwrap_or("1m");
                let default_cur = SettingsRepository::get_or_default(pool, owner_id)
                    .await
                    .currency;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur);
                let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;

                let history =
                    HistoryRepository::get_history(pool, owner_id, timeframe, &currency)
                        .await
                        .map_err(|e| e.to_string())?;

                serde_json::to_string(&history).map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    let mut defs = vec![
        ToolDefinition::function(
            "get_my_portfolio_summary",
            "Fetch a complete summary of the user's investment portfolio, including total value, unrealized P&L, cash balances, and holdings.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "buy_portfolio_holding",
            "Execute a buy order for an asset (stock, ETF, crypto) in the portfolio, deducting the total purchase cost from available cash.",
            json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Ticker symbol or asset identifier (required, e.g. AAPL, BTC)"
                    },
                    "quantity": {
                        "type": "number",
                        "description": "Quantity/shares bought (required, positive decimal)"
                    },
                    "price": {
                        "type": "number",
                        "description": "Unit price paid per share/unit (required, positive decimal)"
                    },
                    "currency": {
                        "type": "string",
                        "description": "Currency code for the transaction"
                    },
                    "note": {
                        "type": "string",
                        "description": "Optional notes or tags for this transaction"
                    }
                },
                "required": ["symbol", "quantity", "price"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_portfolio_holding",
            "Fetch details of a specific holding in the user's portfolio by ticker symbol.",
            json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Ticker symbol (required, e.g. AAPL, BTC)"
                    }
                },
                "required": ["symbol"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "sell_portfolio_holding",
            "Execute a sell order for an existing portfolio holding, realizing P&L and returning the proceeds to portfolio cash.",
            json!({
                "type": "object",
                "properties": {
                    "symbol": {
                        "type": "string",
                        "description": "Ticker symbol (required, e.g. AAPL, BTC)"
                    },
                    "quantity": {
                        "type": "number",
                        "description": "Quantity to sell (required, positive decimal)"
                    },
                    "price": {
                        "type": "number",
                        "description": "Unit price received (required, positive decimal)"
                    }
                },
                "required": ["symbol", "quantity", "price"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_portfolio_performance_history",
            "Retrieve historical portfolio valuation snapshots over a specified timeframe.",
            json!({
                "type": "object",
                "properties": {
                    "timeframe": {
                        "type": "string",
                        "enum": ["1d", "1w", "1m", "3m", "1y", "all"],
                        "description": "Historical time window (default '1m')"
                    },
                    "currency": {
                        "type": "string",
                        "description": "Currency code for valuations"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
    ];
    defs.extend(cash::tool_definitions());
    defs.extend(alerts::tool_definitions());
    defs
}
