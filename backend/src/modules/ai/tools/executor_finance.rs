use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde_json::json;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::models::Direction;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_finance_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_financial_summary" => Some(async {
            let from = Utc::now() - Duration::days(365);
            let to = Utc::now() + Duration::days(1);
            let summary = FinanceRepository::summary(pool, owner_id, from, to)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&summary).map_err(|e| e.to_string())
        }.await),

        "list_finance_transactions" => Some(async {
            let dir = args.get("direction").and_then(|v| v.as_str());
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(10).clamp(1, 50);
            let target_category = args.get("category").and_then(|v| v.as_str()).map(|s| s.to_lowercase());
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(365).max(1);

            let cutoff = Utc::now() - Duration::days(days);
            let items = FinanceRepository::list(pool, owner_id, dir, limit * 3, 0)
                .await
                .map_err(|e| e.to_string())?;

            let filtered: Vec<_> = items
                .into_iter()
                .filter(|tx| tx.occurred_at >= cutoff)
                .filter(|tx| {
                    if let Some(ref cat) = target_category {
                        tx.category_name.as_ref().map(|c| c.to_lowercase()).as_deref() == Some(cat.as_str())
                    } else {
                        true
                    }
                })
                .take(limit as usize)
                .collect();

            serde_json::to_string(&filtered).map_err(|e| e.to_string())
        }.await),

        "get_spending_by_category" => Some(async {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(30).max(1);
            let from = Utc::now() - Duration::days(days);
            let to = Utc::now() + Duration::days(1);

            let summaries = FinanceRepository::summary(pool, owner_id, from, to)
                .await
                .map_err(|e| e.to_string())?;

            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let target_cur = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur).to_uppercase();

            let matched = summaries.into_iter().find(|s| s.currency == target_cur);
            match matched {
                Some(s) => {
                    let total_expense = s.expense_minor;
                    let categories: Vec<_> = s.categories.into_iter().map(|c| {
                        let pct = if total_expense > 0 {
                            (c.expense_minor as f64 / total_expense as f64) * 100.0
                        } else {
                            0.0
                        };
                        json!({
                            "category": c.category.unwrap_or_else(|| "Uncategorized".to_string()),
                            "expense_minor": c.expense_minor,
                            "income_minor": c.income_minor,
                            "expense_percentage": format!("{:.1}%", pct),
                        })
                    }).collect();

                    Ok(json!({
                        "currency": s.currency,
                        "currency_exponent": s.currency_exponent,
                        "timeframe_days": days,
                        "total_expense_minor": total_expense,
                        "total_income_minor": s.income_minor,
                        "net_savings_minor": s.net_minor,
                        "categories": categories,
                    }).to_string())
                }
                None => Ok(json!({
                    "currency": target_cur,
                    "timeframe_days": days,
                    "total_expense_minor": 0,
                    "total_income_minor": 0,
                    "categories": []
                }).to_string()),
            }
        }.await),

        "record_finance_entry" => Some(async {
            let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                format!("{:.2}", num)
            } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                s.to_string()
            } else {
                return Err("Field 'amount' is required".to_string());
            };
            let dir_str = args.get("direction").and_then(|v| v.as_str()).ok_or("Field 'direction' is required (expense or income)")?;
            let direction = Direction::parse(dir_str).ok_or("Invalid direction (must be expense or income)")?;
            let note = args.get("note").and_then(|v| v.as_str()).ok_or("Field 'note' is required")?;

            let default_cur = SettingsRepository::get_or_default(pool, owner_id).await.currency;
            let raw_currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or(&default_cur);
            let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
            let exp = exponent_for(&currency);
            let amount_minor = parse_amount(&amount_str, exp).map_err(|e| e.to_string())?;

            let category_id = if let Some(cat_name) = args.get("category").and_then(|v| v.as_str()) {
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

            let occurred_at = if let Some(date_str) = args.get("date").and_then(|v| v.as_str()) {
                if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                    dt.with_timezone(&Utc)
                } else if let Ok(naive_date) = NaiveDate::parse_from_str(date_str.trim(), "%Y-%m-%d") {
                    naive_date.and_hms_opt(12, 0, 0).map(|d| d.and_utc()).unwrap_or_else(Utc::now)
                } else {
                    Utc::now()
                }
            } else {
                Utc::now()
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
                None,
                None,
                occurred_at,
            )
            .await
            .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TransactionCreated(tx.clone()));
            }
            serde_json::to_string(&tx).map_err(|e| e.to_string())
        }.await),

        "delete_finance_entry" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let tx = FinanceRepository::find_owned(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Transaction #{} not found or not owned by user", id))?;

            let deleted = FinanceRepository::delete(pool, tx.id)
                .await
                .map_err(|e| e.to_string())?;

            if deleted {
                if let Some(b) = broadcaster {
                    b.send(SystemEvent::TransactionDeleted {
                        id: tx.id,
                        room_id: tx.room_id.clone(),
                        owner_id: owner_id.to_string(),
                    });
                }
                Ok(json!({ "deleted": true, "id": tx.id }).to_string())
            } else {
                Err(format!("Failed to delete transaction #{}", id))
            }
        }.await),

        _ => None,
    }
}
