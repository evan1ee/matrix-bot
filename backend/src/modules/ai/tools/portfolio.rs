use serde_json::json;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::portfolio::alerts::{AlertCondition, AlertsRepository};
use crate::modules::portfolio::history::HistoryRepository;
use crate::modules::portfolio::models::{
    cost_of, normalise_symbol, WithdrawOutcome, QUANTITY_EXPONENT,
};
use crate::modules::portfolio::repository::{BuyOutcome, PortfolioRepository, SellOutcome};
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_portfolio_summary" => Some(async {
            let summary = PortfolioRepository::summary(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&summary).map_err(|e| e.to_string())
        }.await),

        "get_portfolio_holding" => Some(async {
            let raw_symbol = args.get("symbol").and_then(|v| v.as_str()).ok_or("Field 'symbol' is required")?;
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
                        obj["unrealized_pl_pct"] = json!(format!("{:.2}%", pl_pct));
                    }

                    Ok(obj.to_string())
                }
                None => Ok(json!({
                    "found": false,
                    "symbol": symbol,
                    "message": format!("No open position found for symbol '{}'", symbol)
                }).to_string()),
            }
        }.await),

        "deposit_portfolio_cash" => Some(async {
            let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'amount' is required".to_string());
            };
            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
            let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
            let exp = exponent_for(&currency);
            let amount_minor = parse_amount(&amount_str, exp).map_err(|e| e.to_string())?;

            let cash = PortfolioRepository::deposit_cash(pool, owner_id, &currency, exp as i64, amount_minor)
                .await
                .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::PortfolioCashUpdated {
                    owner_id: owner_id.to_string(),
                    currency: cash.currency.clone(),
                    balance_minor: cash.balance_minor,
                });
            }
            serde_json::to_string(&cash).map_err(|e| e.to_string())
        }.await),

        "withdraw_portfolio_cash" => Some(async {
            let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'amount' is required".to_string());
            };
            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
            let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
            let exp = exponent_for(&currency);
            let amount_minor = parse_amount(&amount_str, exp).map_err(|e| e.to_string())?;

            let outcome = PortfolioRepository::withdraw_cash(pool, owner_id, &currency, amount_minor)
                .await
                .map_err(|e| e.to_string())?;

            match outcome {
                WithdrawOutcome::Ok(cash) => {
                    if let Some(b) = broadcaster {
                        b.send(SystemEvent::PortfolioCashUpdated {
                            owner_id: owner_id.to_string(),
                            currency: cash.currency.clone(),
                            balance_minor: cash.balance_minor,
                        });
                    }
                    serde_json::to_string(&cash).map_err(|e| e.to_string())
                }
                WithdrawOutcome::InsufficientFunds { available_minor } => {
                    Err(format!("Insufficient cash in {}: only {} minor units available", currency, available_minor))
                }
            }
        }.await),

        "buy_portfolio_holding" => Some(async {
            let raw_symbol = args.get("symbol").and_then(|v| v.as_str()).ok_or("Field 'symbol' is required")?;
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

            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
            let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
            let exp = exponent_for(&currency);
            let price_minor = parse_amount(&price_str, exp).map_err(|e| e.to_string())?;
            let quantity_minor = parse_amount(&qty_str, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;

            let outcome = PortfolioRepository::buy(
                pool,
                owner_id,
                &symbol,
                quantity_minor,
                price_minor,
                &currency,
                exp as i64,
                None,
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
                    Ok(json!({ "status": "bought", "holding": holding, "cash": cash }).to_string())
                }
                BuyOutcome::CurrencyMismatch { existing_currency } => {
                    Err(format!("Holding {} is already priced in {}, cannot buy in {}", symbol, existing_currency, currency))
                }
                BuyOutcome::InsufficientCash { available_minor, needed_minor } => {
                    Err(format!("Insufficient cash: needed {} minor units, but only {} available", needed_minor, available_minor))
                }
            }
        }.await),

        "sell_portfolio_holding" => Some(async {
            let raw_symbol = args.get("symbol").and_then(|v| v.as_str()).ok_or("Field 'symbol' is required")?;
            let symbol = normalise_symbol(raw_symbol)?;

            let existing = PortfolioRepository::find(pool, owner_id, &symbol)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("No position found in '{}' to sell", symbol))?;

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

            let price_minor = parse_amount(&price_str, existing.currency_exponent as u32).map_err(|e| e.to_string())?;
            let quantity_minor = parse_amount(&qty_str, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;

            let outcome = PortfolioRepository::sell(pool, owner_id, &symbol, quantity_minor, price_minor)
                .await
                .map_err(|e| e.to_string())?;

            match outcome {
                SellOutcome::Closed { cash, realized_pl_minor } => {
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
                        "message": format!("Fully sold {} position; proceeds credited to cash.", symbol)
                    }).to_string())
                }
                SellOutcome::Reduced { holding, cash, realized_pl_minor } => {
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
                SellOutcome::ExceedsHolding { held_minor } => {
                    Err(format!("Cannot sell more than held: you hold {} minor units, tried to sell {}", held_minor, quantity_minor))
                }
            }
        }.await),

        "create_portfolio_price_alert" => Some(async {
            let raw_symbol = args.get("symbol").and_then(|v| v.as_str()).ok_or("Field 'symbol' is required")?;
            let symbol = normalise_symbol(raw_symbol)?;

            let cond_str = args.get("condition").and_then(|v| v.as_str()).unwrap_or("above");
            let condition = match cond_str.to_lowercase().as_str() {
                "gte" | "above" | ">" | ">=" | "higher" => AlertCondition::Above,
                "lte" | "below" | "<" | "<=" | "lower" => AlertCondition::Below,
                _ => return Err("Invalid condition: must be 'above' or 'below'".to_string()),
            };

            let price_str = if let Some(num) = args.get("target_price").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = args.get("target_price").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'target_price' is required".to_string());
            };

            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let holding = PortfolioRepository::find(pool, owner_id, &symbol).await.map_err(|e| e.to_string())?;
            let currency = holding.as_ref().map(|h| h.currency.clone()).unwrap_or(default_cur);
            let exp = exponent_for(&currency);
            let target_price_minor = parse_amount(&price_str, exp).map_err(|e| e.to_string())?;

            let alert = AlertsRepository::create(
                pool,
                owner_id,
                &symbol,
                target_price_minor,
                &currency,
                exp as i32,
                condition,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::PortfolioAlertCreated(alert.clone()));
            }
            serde_json::to_string(&alert).map_err(|e| e.to_string())
        }.await),

        "get_portfolio_price_alerts" => Some(async {
            let active_only = args.get("active_only").and_then(|v| v.as_bool()).unwrap_or(false);
            let alerts = AlertsRepository::list_for_owner(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;

            let filtered: Vec<_> = if active_only {
                alerts.into_iter().filter(|a| !a.is_triggered).collect()
            } else {
                alerts
            };
            serde_json::to_string(&filtered).map_err(|e| e.to_string())
        }.await),

        "cancel_portfolio_price_alert" => Some(async {
            if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                let deleted = AlertsRepository::delete(pool, id, owner_id).await.map_err(|e| e.to_string())?;
                if deleted {
                    if let Some(b) = broadcaster {
                        b.send(SystemEvent::PortfolioAlertDeleted { id, owner_id: owner_id.to_string() });
                    }
                    Ok(json!({ "cancelled": true, "id": id }).to_string())
                } else {
                    Err(format!("Alert #{} not found or not owned by user", id))
                }
            } else if let Some(sym_str) = args.get("symbol").and_then(|v| v.as_str()) {
                let symbol = normalise_symbol(sym_str)?;
                let alerts = AlertsRepository::list_for_owner(pool, owner_id).await.map_err(|e| e.to_string())?;
                let to_cancel: Vec<_> = alerts.into_iter().filter(|a| a.symbol == symbol && !a.is_triggered).collect();
                let count = to_cancel.len();
                for a in to_cancel {
                    let _ = AlertsRepository::delete(pool, a.id, owner_id).await;
                    if let Some(b) = broadcaster {
                        b.send(SystemEvent::PortfolioAlertDeleted { id: a.id, owner_id: owner_id.to_string() });
                    }
                }
                Ok(json!({ "cancelled": true, "symbol": symbol, "count": count }).to_string())
            } else {
                Err("Either 'id' or 'symbol' must be provided to cancel an alert".to_string())
            }
        }.await),

        "get_portfolio_performance_history" => Some(async {
            let timeframe = args.get("timeframe").and_then(|v| v.as_str()).unwrap_or("1m");
            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
            let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;

            let history = HistoryRepository::get_history(pool, owner_id, timeframe, &currency)
                .await
                .map_err(|e| e.to_string())?;

            serde_json::to_string(&history).map_err(|e| e.to_string())
        }.await),

        _ => None,
    }
}
