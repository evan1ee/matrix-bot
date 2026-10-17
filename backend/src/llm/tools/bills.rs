use crate::db::DbPool;
use chrono::Utc;
use serde_json::json;

use crate::domain::accounts::AccountId;
use crate::domain::recurring_bills::NewRecurringBill;
use crate::infrastructure::inference::llm::ToolDefinition;

pub async fn execute_tool(
    services: &crate::application::AppServices,
    _pool: &DbPool,
    owner_id: &AccountId,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "evaluate_bill_due_adequacy" => Some(
            async {
                let acct = *owner_id;
                let days_ahead = args
                    .get("days_ahead")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(14)
                    .clamp(1, 60);
                let settings = services.settings.get_or_default(&acct).await;
                let today = Utc::now().with_timezone(&settings.tz()).date_naive();
                let bills = services
                    .recurring_bills
                    .list_by_owner(&acct)
                    .await
                    .map_err(|e| e.to_string())?;
                let cash_balances = services
                    .portfolio
                    .list_cash(&acct)
                    .await
                    .map_err(|e| e.to_string())?;
                let available_cash_minor =
                    cash_balances.iter().map(|c| c.balance_minor).sum::<i64>();
                let report = crate::domain::recurring_bills::due::evaluate_bill_cash_adequacy(
                    &bills,
                    available_cash_minor,
                    today,
                    days_ahead,
                );
                serde_json::to_string(&report).map_err(|e| e.to_string())
            }
            .await,
        ),

        "get_my_recurring_bills" => Some(
            async {
                let acct = *owner_id;
                let items = services
                    .recurring_bills
                    .list_by_owner(&acct)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            .await,
        ),

        "create_recurring_bill" | "add_bill" => Some(
            async {
                let name_val = args
                    .get("name")
                    .or_else(|| args.get("title"))
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'name' (or 'title') is required")?;
                let amount_str = if let Some(num) = args.get("amount").and_then(|v| v.as_f64()) {
                    crate::domain::finance::money::format_decimal_f64(num)
                        .map_err(|e| e.to_string())?
                } else if let Some(s) = args.get("amount").and_then(|v| v.as_str()) {
                    s.to_string()
                } else {
                    return Err("Field 'amount' is required".to_string());
                };
                let acct = *owner_id;
                let settings = services.settings.get_or_default(&acct).await;
                let raw_currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&settings.currency);
                let currency = crate::domain::finance::money::normalise_currency(raw_currency)
                    .map_err(|e| e.to_string())?;
                let exponent = crate::domain::finance::money::exponent_for(&currency);
                let amount_minor =
                    crate::domain::finance::money::parse_amount(&amount_str, exponent)
                        .map_err(|e| e.to_string())?;
                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("subscriptions");
                let frequency = args
                    .get("frequency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("monthly");
                let due_day = args
                    .get("due_day")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(1)
                    .clamp(1, 31) as i32;
                let auto_pay = args
                    .get("auto_pay")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let notes = args
                    .get("notes")
                    .or_else(|| args.get("note"))
                    .or_else(|| args.get("description"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let next_due_at = args
                    .get("next_due_at")
                    .or_else(|| args.get("due_date"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| {
                        Utc::now()
                            .with_timezone(&settings.tz())
                            .format("%Y-%m-%d")
                            .to_string()
                    });

                let (bill, _) = services
                    .recurring_bills
                    .create(
                        &acct,
                        NewRecurringBill {
                            room_id: room_id.to_string(),
                            sender_name: "AI Assistant".to_string(),
                            name: name_val.to_string(),
                            category: category.to_string(),
                            amount_minor,
                            currency,
                            currency_exponent: i64::from(exponent),
                            frequency: frequency.to_string(),
                            due_day: Some(due_day),
                            next_due_at,
                            total_installments: None,
                            auto_pay,
                            notes,
                            remind_before_minutes: None,
                            remind_room_id: None,
                        },
                    )
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&bill).map_err(|e| e.to_string())
            }
            .await,
        ),

        "delete_recurring_bill" => Some(
            async {
                let id = args
                    .get("id")
                    .and_then(|v| v.as_i64())
                    .ok_or("Field 'id' is required")?;
                let acct = *owner_id;
                services
                    .recurring_bills
                    .delete(&acct, id)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&json!({
                    "id": id,
                    "deleted": true
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        "pay_recurring_bill" => Some(
            async {
                let acct = *owner_id;
                let bill_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                    id
                } else if let Some(search) = args
                    .get("name_search")
                    .or_else(|| args.get("name"))
                    .and_then(|v| v.as_str())
                {
                    let bills = services
                        .recurring_bills
                        .list_by_owner(&acct)
                        .await
                        .map_err(|e| e.to_string())?;
                    let matched = bills
                        .into_iter()
                        .find(|b| b.name.to_lowercase().contains(&search.to_lowercase()));
                    match matched {
                        Some(b) => b.id,
                        None => {
                            return Err(format!(
                                "No recurring bill matching '{}' was found",
                                search
                            ))
                        }
                    }
                } else {
                    return Err(
                        "Either 'id', 'name_search', or 'name' is required to pay a recurring bill"
                            .to_string(),
                    );
                };

                let (updated_bill, tx, _) = services
                    .recurring_bills
                    .mark_paid(&acct, bill_id)
                    .await
                    .map_err(|e| e.to_string())?;

                serde_json::to_string(&json!({
                    "bill": updated_bill,
                    "transaction_recorded": tx,
                }))
                .map_err(|e| e.to_string())
            }
            .await,
        ),

        _ => None,
    }
}

pub fn tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition::function(
            "get_my_recurring_bills",
            "Fetch recurring bills and subscriptions, including amounts, recurrence patterns, and next due dates.",
            json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "create_recurring_bill",
            "Create a new recurring subscription or bill schedule with automated payment reminders.",
            json!({
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Name or title of the recurring bill/subscription (required)"
                    },
                    "amount": {
                        "type": "number",
                        "description": "Decimal amount to pay each cycle (required)"
                    },
                    "currency": {
                        "type": "string",
                        "description": "3-letter currency code (e.g. USD, EUR, TWD)"
                    },
                    "frequency": {
                        "type": "string",
                        "enum": ["daily", "weekly", "monthly", "yearly"],
                        "description": "Billing recurrence cycle (required)"
                    },
                    "due_date": {
                        "type": "string",
                        "description": "First/next due date in ISO 8601 or YYYY-MM-DD"
                    },
                    "category": {
                        "type": "string",
                        "description": "Optional category"
                    }
                },
                "required": ["title", "amount", "frequency"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "delete_recurring_bill",
            "Delete a recurring bill schedule by its numeric ID.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the recurring bill to delete (required)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "pay_recurring_bill",
            "Mark a recurring bill/subscription as paid for the current cycle, advancing its next due date and recording an expense transaction.",
            json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "integer",
                        "description": "The numeric ID of the recurring bill to record payment for (required)"
                    },
                    "paid_date": {
                        "type": "string",
                        "description": "Optional payment date in YYYY-MM-DD or ISO 8601 (defaults to current time)"
                    },
                    "record_expense": {
                        "type": "boolean",
                        "description": "Whether to also record an expense transaction in Finance module (defaults to true)"
                    }
                },
                "required": ["id"],
                "additionalProperties": false
            }),
        ),
        ToolDefinition::function(
            "evaluate_bill_due_adequacy",
            "Evaluate upcoming recurring bills due within a planning horizon (default 14 days) and calculate cash flow adequacy against available cash balances.",
            json!({
                "type": "object",
                "properties": {
                    "days_ahead": {
                        "type": "integer",
                        "description": "Planning horizon in days ahead to evaluate bills against available cash (default: 14, range: 1..60)."
                    }
                },
                "additionalProperties": false
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::DbPool;
    use crate::domain::recurring_bills::due::CashFlowAdequacyReport;

    async fn setup() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[test]
    fn test_evaluate_bill_due_adequacy_schema() {
        let defs = tool_definitions();
        let due_def = defs
            .iter()
            .find(|d| d.function.name == "evaluate_bill_due_adequacy")
            .expect("evaluate_bill_due_adequacy tool definition must exist");
        let props = due_def.function.parameters.get("properties").unwrap();
        assert!(props.get("days_ahead").is_some());
    }

    #[tokio::test]
    async fn test_execute_evaluate_bill_due_adequacy() {
        let pool = setup().await;
        let alice = AccountId::from_seed("@alice_bills_due:test");
        crate::db::pool::seed_account(&pool, &alice).await;
        let services = crate::application::AppServices::new(&pool);

        // Deposit cash: $50
        services
            .portfolio
            .deposit_cash(&alice, "USD", 2, 5000, None)
            .await
            .unwrap();

        // Create recurring bill due in 3 days: $100
        let due_date = (Utc::now() + chrono::Duration::days(3))
            .format("%Y-%m-%d")
            .to_string();

        let (bill, _) = services
            .recurring_bills
            .create(
                &alice,
                NewRecurringBill {
                    room_id: "!room:1".to_string(),
                    sender_name: "Alice".to_string(),
                    name: "Server Hosting".to_string(),
                    amount_minor: 10000,
                    currency: "USD".to_string(),
                    currency_exponent: 2,
                    frequency: "monthly".to_string(),
                    due_day: Some(15),
                    category: "infrastructure".to_string(),
                    auto_pay: false,
                    notes: None,
                    next_due_at: due_date,
                    total_installments: None,
                    remind_before_minutes: None,
                    remind_room_id: None,
                },
            )
            .await
            .unwrap();

        let result = execute_tool(
            &services,
            &pool,
            &alice,
            "!room:1",
            "evaluate_bill_due_adequacy",
            &json!({ "days_ahead": 7 }),
        )
        .await
        .expect("evaluate_bill_due_adequacy must be handled")
        .expect("tool execution should succeed");

        let report: CashFlowAdequacyReport =
            serde_json::from_str(&result).expect("valid CashFlowAdequacyReport JSON");

        assert_eq!(report.total_due_minor, 10000);
        assert_eq!(report.available_cash_minor, 5000);
        assert_eq!(report.projected_shortfall_minor, 5000);
        assert!(report.has_shortfall);
        assert_eq!(report.bills.len(), 1);
        assert_eq!(report.bills[0].bill_id, bill.id);
    }
}
