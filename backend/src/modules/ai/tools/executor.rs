use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use sqlx::SqlitePool;
use tracing::{debug, warn};

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::models::Direction;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::portfolio::models::{normalise_symbol, QUANTITY_EXPONENT};
use crate::modules::portfolio::repository::{BuyOutcome, PortfolioRepository};
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;
use crate::modules::todo::models::TodoPriority;
use crate::modules::todo::repository::TodoRepository;

pub async fn execute(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args_json: &str,
) -> Result<String, String> {
    debug!("Executing AI tool '{}' for owner '{}' with args: {}", name, owner_id, args_json);
    let parsed_args: serde_json::Value = if args_json.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(args_json).map_err(|e| format!("Invalid JSON arguments: {}", e))?
    };

    match name {
        "get_my_tasks" => {
            let completed = parsed_args.get("completed").and_then(|v| v.as_bool());
            let items = TodoRepository::list_all_user(pool, owner_id, completed, 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }
        "get_my_reminders" => {
            let items = ReminderRepository::list_user_pending(pool, owner_id, 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }
        "get_my_recurring_bills" => {
            let items = RecurringBillRepository::list_by_owner(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }
        "get_my_financial_summary" => {
            let from = Utc::now() - Duration::days(365);
            let to = Utc::now() + Duration::days(1);
            let summary = FinanceRepository::summary(pool, owner_id, from, to)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&summary).map_err(|e| e.to_string())
        }
        "get_my_portfolio_summary" => {
            let summary = PortfolioRepository::summary(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&summary).map_err(|e| e.to_string())
        }
        "get_my_settings" => {
            let settings = SettingsRepository::get_or_default(pool, owner_id).await;
            serde_json::to_string(&settings).map_err(|e| e.to_string())
        }
        "add_todo" => {
            let text = parsed_args.get("text").and_then(|v| v.as_str()).ok_or("Field 'text' is required")?;
            let priority_str = parsed_args.get("priority").and_then(|v| v.as_str()).unwrap_or("medium");
            let priority = TodoPriority::from_str_or_default(priority_str);
            let due_at = if let Some(mins) = parsed_args.get("due_in_minutes").and_then(|v| v.as_i64()) {
                Some(Utc::now() + Duration::minutes(mins))
            } else if let Some(due_str) = parsed_args.get("due_at").and_then(|v| v.as_str()) {
                DateTime::parse_from_rfc3339(due_str).map(|dt| dt.with_timezone(&Utc)).ok()
            } else {
                None
            };
            let remind_before = parsed_args.get("remind_before_minutes").and_then(|v| v.as_i64());

            let item = TodoRepository::add(
                pool,
                room_id,
                owner_id,
                "AI Assistant",
                text,
                priority,
                due_at,
                remind_before,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TodoCreated(item.clone()));
            }
            serde_json::to_string(&item).map_err(|e| e.to_string())
        }
        "complete_todo" => {
            let target_id = if let Some(id) = parsed_args.get("id").and_then(|v| v.as_i64()) {
                id
            } else if let Some(search) = parsed_args.get("text_search").and_then(|v| v.as_str()) {
                let active_tasks = TodoRepository::list_all_user(pool, owner_id, Some(false), 50, 0)
                    .await
                    .map_err(|e| e.to_string())?;
                let matched = active_tasks.into_iter().find(|t| t.text.to_lowercase().contains(&search.to_lowercase()));
                match matched {
                    Some(t) => t.id,
                    None => return Err(format!("No active task matching '{}' was found", search)),
                }
            } else {
                return Err("Either 'id' or 'text_search' must be provided to complete a task".to_string());
            };

            let item = TodoRepository::find_owned(pool, target_id, owner_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task #{} not found or not owned by user", target_id))?;

            let updated = TodoRepository::set_completed(pool, item.id, true)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Failed to complete task".to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TodoUpdated(updated.clone()));
            }
            serde_json::to_string(&updated).map_err(|e| e.to_string())
        }
        "record_finance_entry" => {
            let amount_str = if let Some(num) = parsed_args.get("amount").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = parsed_args.get("amount").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'amount' is required".to_string());
            };
            let dir_str = parsed_args.get("direction").and_then(|v| v.as_str()).ok_or("Field 'direction' is required (expense or income)")?;
            let direction = Direction::parse(dir_str).ok_or("Invalid direction (must be expense or income)")?;
            let note = parsed_args.get("note").and_then(|v| v.as_str()).ok_or("Field 'note' is required")?;

            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = parsed_args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
            let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
            let exp = exponent_for(&currency);
            let amount_minor = parse_amount(&amount_str, exp).map_err(|e| e.to_string())?;

            let category_id = if let Some(cat_name) = parsed_args.get("category").and_then(|v| v.as_str()) {
                if let Some(existing) = FinanceRepository::find_category_by_name(pool, owner_id, cat_name).await.map_err(|e| e.to_string())? {
                    Some(existing.id)
                } else if let Some(new_cat) = FinanceRepository::create_category(pool, owner_id, cat_name).await.map_err(|e| e.to_string())? {
                    Some(new_cat.id)
                } else {
                    None
                }
            } else {
                None
            };

            let tx = FinanceRepository::create_transaction(
                pool,
                room_id,
                owner_id,
                "AI Assistant",
                direction.as_str(),
                amount_minor,
                &currency,
                exp as i64,
                category_id,
                Some(note),
                Utc::now(),
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TransactionCreated(tx.clone()));
            }
            serde_json::to_string(&tx).map_err(|e| e.to_string())
        }
        "set_reminder" => {
            let message = parsed_args.get("message").and_then(|v| v.as_str()).ok_or("Field 'message' is required")?;
            let rec_str = parsed_args.get("recurrence").and_then(|v| v.as_str()).unwrap_or("none");
            let recurrence = RecurrencePattern::from_str_or_default(rec_str);

            let trigger_at = if let Some(mins) = parsed_args.get("in_minutes").and_then(|v| v.as_i64()) {
                Utc::now() + Duration::minutes(mins)
            } else if let Some(trig_str) = parsed_args.get("trigger_at").and_then(|v| v.as_str()) {
                DateTime::parse_from_rfc3339(trig_str)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|e| format!("Invalid trigger_at timestamp: {}", e))?
            } else {
                return Err("Either 'in_minutes' or 'trigger_at' must be provided for a reminder".to_string());
            };

            let rem = ReminderRepository::create(
                pool,
                room_id,
                owner_id,
                "AI Assistant",
                message,
                trigger_at,
                recurrence,
                None,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::ReminderCreated(rem.clone()));
            }
            serde_json::to_string(&rem).map_err(|e| e.to_string())
        }
        "deposit_portfolio_cash" => {
            let amount_str = if let Some(num) = parsed_args.get("amount").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = parsed_args.get("amount").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'amount' is required".to_string());
            };
            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = parsed_args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
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
        }
        "buy_portfolio_holding" => {
            let raw_symbol = parsed_args.get("symbol").and_then(|v| v.as_str()).ok_or("Field 'symbol' is required")?;
            let symbol = normalise_symbol(raw_symbol)?;

            let qty_str = if let Some(num) = parsed_args.get("quantity").and_then(|v| v.as_f64()) {
                num.to_string()
            } else if let Some(s) = parsed_args.get("quantity").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'quantity' is required".to_string());
            };

            let price_str = if let Some(num) = parsed_args.get("price").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = parsed_args.get("price").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'price' is required".to_string());
            };

            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = parsed_args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
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
                    Ok(json!({ "holding": holding, "cash": cash }).to_string())
                }
                BuyOutcome::CurrencyMismatch { existing_currency } => {
                    Err(format!("Holding {} is already priced in {}, cannot buy in {}", symbol, existing_currency, currency))
                }
                BuyOutcome::InsufficientCash { available_minor, needed_minor } => {
                    Err(format!("Insufficient cash: needed {} minor units, but only {} available", needed_minor, available_minor))
                }
            }
        }
        unknown => {
            warn!("Unknown AI tool requested: {}", unknown);
            Err(format!("Tool '{}' not found", unknown))
        }
    }
}
