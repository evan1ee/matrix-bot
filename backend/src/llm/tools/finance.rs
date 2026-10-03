use crate::db::DbPool;
use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use serde_json::json;

use crate::domain::accounts::AccountId;
use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::domain::finance::{Direction, ListTransactionsFilter, NewTransaction};
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    _pool: &DbPool,
    owner_id: &AccountId,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    let owner_acct = *owner_id;
    match name {
        "get_my_financial_summary" => Some(
            async {
                let from = Utc::now() - Duration::days(365);
                let to = Utc::now() + Duration::days(1);
                let summary = services
                    .finance
                    .summary(&owner_acct, from, to)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&summary).map_err(|e| e.to_string())
            }
            .await,
        ),

        "list_finance_transactions" => Some(
            async {
                let dir = args
                    .get("direction")
                    .or_else(|| args.get("entry_type"))
                    .or_else(|| args.get("type"))
                    .and_then(|v| v.as_str());
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(10)
                    .clamp(1, 50);
                let target_category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_lowercase());
                let days = args
                    .get("days")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(365)
                    .max(1);

                let cutoff = Utc::now() - Duration::days(days);
                let filter = ListTransactionsFilter {
                    direction: dir.and_then(Direction::parse),
                    limit: Some(limit * 3),
                    ..Default::default()
                };
                let items = services
                    .finance
                    .list_transactions(&owner_acct, &filter)
                    .await
                    .map_err(|e| e.to_string())?;

                let filtered: Vec<_> = items
                    .into_iter()
                    .filter(|tx| tx.occurred_at >= cutoff)
                    .filter(|tx| {
                        if let Some(ref cat) = target_category {
                            tx.category_name
                                .as_ref()
                                .map(|c| c.to_lowercase())
                                .as_deref()
                                == Some(cat.as_str())
                        } else {
                            true
                        }
                    })
                    .take(limit as usize)
                    .collect();

                serde_json::to_string(&filtered).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_spending_by_category" => Some(
            async {
                let days = args
                    .get("days")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(30)
                    .max(1);
                let from = Utc::now() - Duration::days(days);
                let to = Utc::now() + Duration::days(1);

                let summaries = services
                    .finance
                    .summary(&owner_acct, from, to)
                    .await
                    .map_err(|e| e.to_string())?;

                let acct = owner_acct;
                let default_cur = services.settings.get_or_default(&acct).await.currency;
                let target_cur = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&default_cur)
                    .to_uppercase();

                let matched = summaries.into_iter().find(|s| s.currency == target_cur);
                match matched {
                    Some(s) => {
                        let total_expense = s.expense_minor;
                        let categories: Vec<_> = s
                            .categories
                            .into_iter()
                            .map(|c| {
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
                            })
                            .collect();

                        Ok(json!({
                            "currency": s.currency,
                            "currency_exponent": s.currency_exponent,
                            "timeframe_days": days,
                            "total_expense_minor": total_expense,
                            "total_income_minor": s.income_minor,
                            "net_savings_minor": s.net_minor,
                            "categories": categories,
                        })
                        .to_string())
                    }
                    None => Ok(json!({
                        "currency": target_cur,
                        "timeframe_days": days,
                        "total_expense_minor": 0,
                        "total_income_minor": 0,
                        "categories": []
                    })
                    .to_string()),
                }
            }
            .await,
        ),

        "record_finance_entry" | "add_expense" => Some(
            async {
                let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                    format!("{:.2}", num)
                } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'amount' is required".to_string());
                };
                let dir_str = args
                    .get("direction")
                    .or_else(|| args.get("entry_type"))
                    .or_else(|| args.get("type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("expense");
                let direction = Direction::parse(dir_str)
                    .ok_or("Invalid direction (must be expense or income)")?;
                let category_name = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("General");
                let note = args
                    .get("description")
                    .or_else(|| args.get("note"))
                    .or_else(|| args.get("item"))
                    .or_else(|| args.get("merchant"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(category_name);

                let acct = *owner_id;
                let settings = services.settings.get_or_default(&acct).await;
                let tz = settings.tz();
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&settings.currency);
                let currency = normalise_currency(raw_currency).map_err(|e| e.to_string())?;
                let exp = exponent_for(&currency);
                let amount_minor = parse_amount(&amount_str, exp).map_err(|e| e.to_string())?;

                let occurred_at = if let Some(date_str) = args
                    .get("occurred_at")
                    .or_else(|| args.get("date"))
                    .or_else(|| args.get("datetime"))
                    .and_then(|v| v.as_str())
                {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(date_str) {
                        dt.with_timezone(&Utc)
                    } else if let Ok(naive_date) =
                        NaiveDate::parse_from_str(date_str.trim(), "%Y-%m-%d")
                    {
                        naive_date
                            .and_hms_opt(12, 0, 0)
                            .and_then(|naive_dt| tz.from_local_datetime(&naive_dt).single())
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(Utc::now)
                    } else {
                        Utc::now()
                    }
                } else {
                    Utc::now()
                };

                let rooms = &services.rooms;
                let target_room = if let Some(r) = args.get("room").and_then(|v| v.as_str()) {
                    rooms.resolve_target_room(&acct, Some(r)).await
                } else if room_id.is_empty() || room_id == "web" || room_id == "web_chat" {
                    rooms.resolve_target_room(&acct, None).await
                } else {
                    room_id.to_string()
                };

                let tx = services
                    .finance
                    .create_transaction(
                        &acct,
                        NewTransaction {
                            direction,
                            amount_minor,
                            currency: &currency,
                            currency_exponent: exp as i64,
                            category: Some(category_name),
                            note: Some(note),
                            receipt_storage_key: None,
                            room_id: &target_room,
                            sender_name: "AI Assistant",
                            occurred_at,
                            source: None,
                            create_missing_category: true,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&tx).map_err(|e| e.to_string())
            }
            .await,
        ),

        "delete_finance_entry" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let tx = services
                    .finance
                    .delete_transaction(&owner_acct, id)
                    .await
                    .map_err(|e| e.to_string())?;

                Ok(json!({ "deleted": true, "id": tx.id }).to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "get_my_financial_summary",
            "Fetch financial summaries including expense and income totals broken down by category and currency. Currencies are separate and must never be summed together without exchange rates.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "list_finance_transactions",
            "List recent financial transactions with optional filters for type (income/expense) or category.",
            json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of transactions to return (default 20, max 100)"
                    },
                    "offset": {
                        "type": "integer",
                        "description": "Offset for pagination"
                    },
                    "entry_type": {
                        "type": "string",
                        "enum": ["income", "expense"],
                        "description": "Optional filter by transaction type"
                    },
                    "category": {
                        "type": "string",
                        "description": "Optional filter by category name"
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "get_spending_by_category",
            "Get a breakdown of expenses grouped by category for a given month (defaults to current month).",
            json!({
                "type": "object",
                "properties": {
                    "month": {
                        "type": "string",
                        "description": "Optional month in YYYY-MM format. Defaults to current month."
                    }
                },
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "record_finance_entry",
            "Record a new financial expense or income transaction with amount, category, currency, and optional description.",
            json!({
                "type": "object",
                "properties": {
                    "entry_type": {
                        "type": "string",
                        "enum": ["income", "expense"],
                        "description": "Type: 'expense' or 'income' (defaults to 'expense')"
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["income", "expense"],
                        "description": "Alias for entry_type"
                    },
                    "amount": {
                        "type": "number",
                        "description": "Decimal monetary amount (required, e.g. 42.50)"
                    },
                    "category": {
                        "type": "string",
                        "description": "Category name, e.g. Food, Shopping, Leisure, Transport"
                    },
                    "description": {
                        "type": "string",
                        "description": "Optional description of the transaction"
                    },
                    "note": {
                        "type": "string",
                        "description": "Alias for description"
                    },
                    "currency": {
                        "type": "string",
                        "description": "Optional currency code (e.g. CNY, USD, EUR)"
                    },
                    "occurred_at": {
                        "type": "string",
                        "description": "Optional ISO-8601 timestamp or YYYY-MM-DD"
                    },
                    "date": {
                        "type": "string",
                        "description": "Alias for occurred_at"
                    },
                    "room": {
                        "type": "string",
                        "description": "Matrix room name or room ID"
                    }
                },
                "required": ["amount"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "delete_finance_entry",
            "Delete an expense or income transaction by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the financial transaction to delete (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::finance::MAX_NOTE_LEN;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    /// The bug this refactor exists to fix: the AI tool skipped validation
    /// entirely, so a note over the limit reached the database while every
    /// other transport rejected it.
    #[tokio::test]
    async fn test_the_ai_tool_enforces_the_note_limit() {
        let pool = setup().await;
        let over_limit_note = "x".repeat(MAX_NOTE_LEN + 1);
        let alice = AccountId::from_seed("@alice:a");
        crate::db::pool::seed_account(&pool, &alice).await;

        let result = execute_tool(
            &crate::application::AppServices::new(&pool),
            &pool,
            &alice,
            "!room:1",
            "record_finance_entry",
            &json!({
                "amount": 10.0,
                "note": over_limit_note,
            }),
        )
        .await
        .expect("record_finance_entry must be handled");

        assert!(result.is_err(), "a note over the limit must be rejected");
    }
}
