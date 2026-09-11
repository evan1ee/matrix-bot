use chrono::{DateTime, Duration, TimeZone, Utc};
use chrono_tz::Tz;
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::recurring_bills::repository::RecurringBillRepository;
use crate::modules::reminder::models::RecurrencePattern;
use crate::modules::reminder::repository::ReminderRepository;
use crate::modules::settings::repository::SettingsRepository;
use crate::modules::todo::models::TodoPriority;
use crate::modules::todo::repository::TodoRepository;

fn parse_due_at(due_str: &str, tz: &Tz) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(due_str) {
        Some(dt.with_timezone(&Utc))
    } else if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(due_str, "%Y-%m-%dT%H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(due_str, "%Y-%m-%d %H:%M:%S"))
    {
        tz.from_local_datetime(&naive_dt).single().map(|dt| dt.with_timezone(&Utc))
    } else if let Ok(naive_date) = chrono::NaiveDate::parse_from_str(due_str.trim(), "%Y-%m-%d") {
        naive_date.and_hms_opt(18, 0, 0)
            .and_then(|dt| tz.from_local_datetime(&dt).single())
            .map(|dt| dt.with_timezone(&Utc))
    } else {
        None
    }
}

pub async fn execute_todo_tool(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    room_id: &str,
    name: &str,
    args: &serde_json::Value,
) -> Option<Result<String, String>> {
    match name {
        "get_my_tasks" => Some(async {
            let completed = args.get("completed").and_then(|v| v.as_bool());
            let items = TodoRepository::list_all_user(pool, owner_id, completed, 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }.await),

        "get_my_reminders" => Some(async {
            let items = ReminderRepository::list_user_pending(pool, owner_id, 50, 0)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }.await),

        "get_my_recurring_bills" => Some(async {
            let items = RecurringBillRepository::list_by_owner(pool, owner_id)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::to_string(&items).map_err(|e| e.to_string())
        }.await),

        "get_my_settings" => Some(async {
            let settings = SettingsRepository::get_or_default(pool, owner_id).await;
            serde_json::to_string(&settings).map_err(|e| e.to_string())
        }.await),

        "add_todo" => Some(async {
            let text = args.get("text").and_then(|v| v.as_str()).ok_or("Field 'text' is required")?;
            let priority_str = args.get("priority").and_then(|v| v.as_str()).unwrap_or("medium");
            let priority = TodoPriority::from_str_or_default(priority_str);
            let due_at = if let Some(mins) = args.get("due_in_minutes").and_then(|v| v.as_i64()) {
                Some(Utc::now() + Duration::minutes(mins))
            } else if let Some(due_str) = args.get("due_at").and_then(|v| v.as_str()) {
                let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                parse_due_at(due_str, &settings.tz())
            } else {
                None
            };
            let remind_before = args.get("remind_before_minutes").and_then(|v| v.as_i64());

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
        }.await),

        "complete_todo" => Some(async {
            let target_id = if let Some(id) = args.get("id").and_then(|v| v.as_i64()) {
                id
            } else if let Some(search) = args.get("text_search").and_then(|v| v.as_str()) {
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
        }.await),

        "set_reminder" => Some(async {
            let message = args.get("message").and_then(|v| v.as_str()).ok_or("Field 'message' is required")?;
            let rec_str = args.get("recurrence").and_then(|v| v.as_str()).unwrap_or("none");
            let recurrence = RecurrencePattern::from_str_or_default(rec_str);

            let trigger_at = if let Some(mins) = args.get("in_minutes").and_then(|v| v.as_i64()) {
                Utc::now() + Duration::minutes(mins)
            } else if let Some(trig_str) = args.get("trigger_at").and_then(|v| v.as_str()) {
                let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                let tz = settings.tz();
                if let Ok(dt) = DateTime::parse_from_rfc3339(trig_str) {
                    dt.with_timezone(&Utc)
                } else if let Ok(naive_dt) = chrono::NaiveDateTime::parse_from_str(trig_str, "%Y-%m-%dT%H:%M:%S")
                    .or_else(|_| chrono::NaiveDateTime::parse_from_str(trig_str, "%Y-%m-%d %H:%M:%S"))
                {
                    tz.from_local_datetime(&naive_dt)
                        .single()
                        .map(|dt| dt.with_timezone(&Utc))
                        .ok_or_else(|| "Ambiguous or invalid local time for trigger_at".to_string())?
                } else {
                    return Err(format!("Invalid trigger_at timestamp: {}", trig_str));
                }
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
        }.await),

        "cancel_reminder" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let rem = ReminderRepository::find_owned(pool, owner_id, id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Reminder #{} not found or not owned by user", id))?;

            let ok = ReminderRepository::cancel(pool, owner_id, id)
                .await
                .map_err(|e| e.to_string())?;

            if !ok {
                return Err(format!("Reminder #{} could not be cancelled", id));
            }

            if let Some(b) = broadcaster {
                b.send(SystemEvent::ReminderCancelled {
                    id,
                    room_id: rem.room_id.clone(),
                    owner_id: owner_id.to_string(),
                });
            }

            serde_json::to_string(&serde_json::json!({
                "id": id,
                "cancelled": true
            })).map_err(|e| e.to_string())
        }.await),

        "update_todo" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let _existing = TodoRepository::find_owned(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task #{} not found or not owned by user", id))?;

            if let Some(text) = args.get("text").and_then(|v| v.as_str()) {
                TodoRepository::edit_text(pool, id, text).await.map_err(|e| e.to_string())?;
            }

            if let Some(priority_str) = args.get("priority").and_then(|v| v.as_str()) {
                let priority = TodoPriority::from_str_or_default(priority_str);
                TodoRepository::set_priority(pool, id, priority).await.map_err(|e| e.to_string())?;
            }

            if args.get("due_in_minutes").is_some() || args.get("due_at").is_some() {
                let due_at = if let Some(mins) = args.get("due_in_minutes").and_then(|v| v.as_i64()) {
                    Some(Utc::now() + Duration::minutes(mins))
                } else if let Some(due_str) = args.get("due_at").and_then(|v| v.as_str()) {
                    let settings = SettingsRepository::get_or_default(pool, owner_id).await;
                    parse_due_at(due_str, &settings.tz())
                } else {
                    None
                };
                TodoRepository::set_due_date(pool, id, due_at).await.map_err(|e| e.to_string())?;
            }

            let updated = TodoRepository::find_owned(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "Failed to retrieve updated task".to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TodoUpdated(updated.clone()));
            }

            serde_json::to_string(&updated).map_err(|e| e.to_string())
        }.await),

        "delete_todo" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let existing = TodoRepository::find_owned(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("Task #{} not found or not owned by user", id))?;

            let ok = TodoRepository::delete(pool, id)
                .await
                .map_err(|e| e.to_string())?;

            if !ok {
                return Err(format!("Failed to delete task #{}", id));
            }

            if let Some(b) = broadcaster {
                b.send(SystemEvent::TodoDeleted {
                    id,
                    room_id: existing.room_id.clone(),
                    owner_id: owner_id.to_string(),
                });
            }

            serde_json::to_string(&serde_json::json!({
                "id": id,
                "deleted": true
            })).map_err(|e| e.to_string())
        }.await),

        "create_recurring_bill" => Some(async {
            let name_val = args.get("name").and_then(|v| v.as_str()).ok_or("Field 'name' is required")?;
            let amount_str = args.get("amount").and_then(|v| v.as_str()).ok_or("Field 'amount' is required")?;
            let currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or("USD").to_uppercase();
            let category = args.get("category").and_then(|v| v.as_str()).unwrap_or("subscriptions");
            let frequency = args.get("frequency").and_then(|v| v.as_str()).unwrap_or("monthly");
            let due_day = args.get("due_day").and_then(|v| v.as_i64()).unwrap_or(1).clamp(1, 31) as i32;
            let auto_pay = args.get("auto_pay").and_then(|v| v.as_bool()).unwrap_or(false);
            let notes = args.get("notes").and_then(|v| v.as_str()).map(|s| s.to_string());

            let amount_f: f64 = amount_str.parse().map_err(|_| "Invalid amount format")?;
            let amount_minor = (amount_f * 100.0).round() as i64;

            let settings = SettingsRepository::get_or_default(pool, owner_id).await;
            let today = Utc::now().with_timezone(&settings.tz()).format("%Y-%m-%d").to_string();
            let payload = crate::modules::recurring_bills::models::CreateBillPayload {
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

            let bill = RecurringBillRepository::create(pool, room_id, owner_id, "AI Assistant", payload)
                .await
                .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillCreated(bill.clone()));
            }

            serde_json::to_string(&bill).map_err(|e| e.to_string())
        }.await),

        "delete_recurring_bill" => Some(async {
            let id = args.get("id").and_then(|v| v.as_i64()).ok_or("Field 'id' is required")?;
            let ok = RecurringBillRepository::delete(pool, id, owner_id)
                .await
                .map_err(|e| e.to_string())?;

            if !ok {
                return Err(format!("Recurring bill #{} not found or not owned by user", id));
            }

            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillDeleted {
                    id,
                    owner_id: owner_id.to_string(),
                });
            }

            serde_json::to_string(&serde_json::json!({
                "id": id,
                "deleted": true
            })).map_err(|e| e.to_string())
        }.await),

        "update_my_settings" => Some(async {
            let mut settings = SettingsRepository::get_or_default(pool, owner_id).await;
            if let Some(tz) = args.get("timezone").and_then(|v| v.as_str()) {
                settings.timezone = tz.to_string();
            }
            if let Some(curr) = args.get("currency").and_then(|v| v.as_str()) {
                settings.currency = curr.to_uppercase();
            }

            let saved = SettingsRepository::upsert(pool, owner_id, &settings)
                .await
                .map_err(|e| e.to_string())?;

            if let Some(b) = broadcaster {
                b.send(SystemEvent::SettingsUpdated {
                    owner_id: owner_id.to_string(),
                    settings: saved.clone(),
                });
            }

            serde_json::to_string(&saved).map_err(|e| e.to_string())
        }.await),

        _ => None,
    }
}
