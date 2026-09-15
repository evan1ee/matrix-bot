use chrono::Utc;
use serde_json::json;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::recurring_bills::models::CreateBillPayload;
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::settings::repository::SettingsRepository;

pub async fn execute_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_recurring_bills" => Some(
            async {
                let items = RecurringBillRepository::list_by_owner(pool, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;
                serde_json::to_string(&items).map_err(|e| e.to_string())
            }
            .await,
        ),

        "create_recurring_bill" => Some(
            async {
                let name_val = args
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'name' is required")?;
                let amount_str = args
                    .get("amount")
                    .and_then(|v| v.as_str())
                    .ok_or("Field 'amount' is required")?;
                let currency = args
                    .get("currency")
                    .and_then(|v| v.as_str())
                    .unwrap_or("USD")
                    .to_uppercase();
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
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                let amount_f: f64 = amount_str.parse().map_err(|_| "Invalid amount format")?;
                let amount_minor = (amount_f * 100.0).round() as i64;

                let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                let today = Utc::now()
                    .with_timezone(&settings.tz())
                    .format("%Y-%m-%d")
                    .to_string();
                let payload = CreateBillPayload {
                    name: name_val.to_string(),
                    category: category.to_string(),
                    amount_minor,
                    currency,
                    currency_exponent: Some(2),
                    frequency: Some(frequency.to_string()),
                    due_day: Some(due_day),
                    next_due_at: today,
                    total_installments: None,
                    auto_pay: Some(auto_pay),
                    notes,
                    remind_before_minutes: None,
                    remind_room_id: None,
                };

                let bill = RecurringBillRepository::create(
                    pool,
                    room_id,
                    owner_id,
                    "AI Assistant",
                    payload,
                )
                .await
                .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::RecurringBillCreated(bill.clone()));
                }

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
                let ok = RecurringBillRepository::delete(pool, id, owner_id)
                    .await
                    .map_err(|e| e.to_string())?;

                if !ok {
                    return Err(format!(
                        "Recurring bill #{} not found or not owned by user",
                        id
                    ));
                }

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::RecurringBillDeleted {
                        id,
                        owner_id: owner_id.to_string(),
                    });
                }

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
                let bill_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                    id
                } else if let Some(search) = args
                    .get("name_search")
                    .or_else(|| args.get("name"))
                    .and_then(|v| v.as_str())
                {
                    let bills = RecurringBillRepository::list_by_owner(pool, owner_id)
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

                let (updated_bill, tx) =
                    RecurringBillRepository::mark_paid(pool, bill_id, owner_id)
                        .await
                        .map_err(|e| e.to_string())?;

                if let Some(b) = broadcaster {
                    b.send(SystemEvent::RecurringBillUpdated(updated_bill.clone()));
                    b.send(SystemEvent::TransactionCreated(tx.clone()));
                }

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
