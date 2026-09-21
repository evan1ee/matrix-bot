use crate::db::Database;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::CommandContext;
use crate::utils::html::html_escape;

use super::due_reminder;
use super::models::{CreateBillPayload, UpdateBillPayload};
use super::repository::RecurringBillRepository;
use crate::modules::due_reminder::SyncOutcome;

pub async fn handle_bill_command(
    ctx: &CommandContext,
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    let args: Vec<&str> = ctx.raw_args.split_whitespace().collect();
    let sub_cmd = args.first().copied().unwrap_or("list").to_lowercase();

    match sub_cmd.as_str() {
        "list" | "" => handle_list(ctx, db).await,
        "add" => handle_add(ctx, db, broadcaster, &args[1..]).await,
        "pay" => handle_pay(ctx, db, broadcaster, &args[1..]).await,
        "delete" | "del" | "rm" => handle_delete(ctx, db, broadcaster, &args[1..]).await,
        "remind" => handle_remind(ctx, db, broadcaster, &args[1..]).await,
        _ => Some(format!(
            "Unknown subcommand <code>{}</code>. Usage: <code>!bill [list|add|pay|delete]</code>",
            html_escape(&sub_cmd)
        )),
    }
}

async fn handle_list(ctx: &CommandContext, db: &Database) -> Option<String> {
    let bills = RecurringBillRepository::list_by_owner(db.sqlite_pool(), &ctx.owner_id)
        .await
        .unwrap_or_default();

    if bills.is_empty() {
        return Some("No recurring bills recorded. Use <code>!bill add &lt;name&gt; &lt;amount&gt; &lt;currency&gt; &lt;category&gt;</code> to create one.".to_string());
    }

    let mut html = String::from("<b>📋 Recurring Bills & Subscriptions</b><br/><ul>");

    for bill in &bills {
        let amount_formatted = format!("{:.2}", bill.amount_minor as f64 / 100.0);
        let category_badge = match bill.category.as_str() {
            "subscription" => "💳 Subscription",
            "fixed_bill" => "🏠 Fixed Bill",
            "installment" => "📦 Installment",
            "recurring_transfer" => "🔄 Recurring Transfer",
            _ => "📄 Other",
        };

        let installment_info = if let Some(total) = bill.total_installments {
            format!(" ({}/{} paid)", bill.paid_installments, total)
        } else {
            String::new()
        };

        html.push_str(&format!(
            "<li><b>#{}</b>: {} - <b>{} {}</b> ({}) [Next due: {}]{}</li>",
            bill.id,
            html_escape(&bill.name),
            amount_formatted,
            html_escape(&bill.currency),
            category_badge,
            html_escape(&bill.next_due_at),
            installment_info
        ));
    }

    html.push_str("</ul>");
    Some(html)
}

async fn handle_add(
    ctx: &CommandContext,
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
    args: &[&str],
) -> Option<String> {
    // Usage: !bill add <name> <amount> <currency> <category> [frequency]
    if args.len() < 4 {
        return Some("Usage: <code>!bill add &lt;name&gt; &lt;amount&gt; &lt;currency&gt; &lt;category&gt; [frequency]</code><br/>Categories: subscription, fixed_bill, installment, recurring_transfer".to_string());
    }

    let name = args[0];
    let amount_str = args[1];
    let currency = args[2].to_uppercase();
    let category = args[3].to_lowercase();
    let frequency = args.get(4).copied().unwrap_or("monthly").to_lowercase();

    let amount: f64 = match amount_str.parse() {
        Ok(v) if v > 0.0 => v,
        _ => return Some("Invalid amount. Must be a positive number.".to_string()),
    };

    let amount_minor = (amount * 100.0).round() as i64;
    let next_due_at = chrono::Utc::now().format("%Y-%m-%d 00:00:00").to_string();

    let payload = CreateBillPayload {
        name: name.to_string(),
        category,
        amount_minor,
        currency: currency.clone(),
        currency_exponent: Some(2),
        frequency: Some(frequency.clone()),
        due_day: None,
        next_due_at: next_due_at.clone(),
        total_installments: None,
        auto_pay: Some(false),
        notes: None,
        remind_before_minutes: None,
        remind_room_id: None,
    };

    match RecurringBillRepository::create(
        db.sqlite_pool(),
        &ctx.room_id,
        &ctx.owner_id,
        &ctx.sender_name,
        payload,
    )
    .await
    {
        Ok(bill) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillCreated(bill.clone()));
            }
            Some(format!(
                "Created recurring bill <b>#{}</b>: {} ({} {:.2})",
                bill.id,
                html_escape(&bill.name),
                html_escape(&bill.currency),
                amount
            ))
        }
        Err(err) => Some(format!("Failed to create bill: {}", err)),
    }
}

async fn handle_pay(
    ctx: &CommandContext,
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
    args: &[&str],
) -> Option<String> {
    if args.is_empty() {
        return Some("Usage: <code>!bill pay &lt;id&gt;</code>".to_string());
    }

    let id: i64 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => return Some("Invalid bill ID.".to_string()),
    };

    match RecurringBillRepository::mark_paid(db.sqlite_pool(), id, &ctx.owner_id, broadcaster).await
    {
        Ok((bill, _tx)) => {
            // Paying advanced `next_due_at`, so re-arm at the next period.
            let _ = due_reminder::sync(db.sqlite_pool(), broadcaster, &bill).await;
            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillPaid {
                    id: bill.id,
                    owner_id: bill.owner_id.clone(),
                    next_due_at: bill.next_due_at.clone(),
                });
                b.send(SystemEvent::RecurringBillUpdated(bill.clone()));
            }
            Some(format!(
                "Marked bill <b>#{}</b> ({}) as paid. Next due: <b>{}</b>",
                bill.id,
                html_escape(&bill.name),
                html_escape(&bill.next_due_at)
            ))
        }
        Err(_) => Some(format!("Bill #{} not found or access denied.", id)),
    }
}

async fn handle_delete(
    ctx: &CommandContext,
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
    args: &[&str],
) -> Option<String> {
    if args.is_empty() {
        return Some("Usage: <code>!bill delete &lt;id&gt;</code>".to_string());
    }

    let id: i64 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => return Some("Invalid bill ID.".to_string()),
    };

    match RecurringBillRepository::delete(db.sqlite_pool(), id, &ctx.owner_id).await {
        Ok(true) => {
            let _ = due_reminder::tear_down(db.sqlite_pool(), broadcaster, id).await;
            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillDeleted {
                    id,
                    owner_id: ctx.owner_id.clone(),
                });
            }
            Some(format!("Deleted bill <b>#{}</b>.", id))
        }
        _ => Some(format!("Bill #{} not found or access denied.", id)),
    }
}

/// `!bill remind <id> <minutes|off>` — sets or clears the lead time.
///
/// Minutes rather than a friendlier "2 days" grammar because that is the unit
/// the column and the REST field already speak; one representation across the
/// two entry points beats a nicer one that disagrees with the API.
async fn handle_remind(
    ctx: &CommandContext,
    db: &Database,
    broadcaster: Option<&EventBroadcaster>,
    args: &[&str],
) -> Option<String> {
    if args.len() < 2 {
        return Some("Usage: <code>!bill remind &lt;id&gt; &lt;minutes|off&gt;</code>".to_string());
    }

    let id: i64 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => return Some("Invalid bill ID.".to_string()),
    };

    let lead = match args[1].to_lowercase().as_str() {
        "off" | "none" | "0" => None,
        raw => match raw.parse::<i64>() {
            Ok(v) if v > 0 && v <= crate::modules::due_reminder::MAX_LEAD_MINUTES => Some(v),
            _ => {
                return Some(format!(
                    "Lead time must be between 1 and {} minutes, or <code>off</code>.",
                    crate::modules::due_reminder::MAX_LEAD_MINUTES
                ))
            }
        },
    };

    let payload = UpdateBillPayload {
        remind_before_minutes: Some(lead),
        ..Default::default()
    };

    match RecurringBillRepository::update(db.sqlite_pool(), id, &ctx.owner_id, payload).await {
        Ok(bill) => {
            let outcome = due_reminder::sync(db.sqlite_pool(), broadcaster, &bill).await;
            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillUpdated(bill.clone()));
            }
            Some(match (lead, outcome) {
                (None, _) => format!("Reminder cleared for bill <b>#{}</b>.", bill.id),
                (Some(m), Ok(SyncOutcome::Scheduled(at))) => format!(
                    "Will remind {} minutes before bill <b>#{}</b> ({}) is due — {} UTC.",
                    m,
                    bill.id,
                    html_escape(&bill.name),
                    at.format("%Y-%m-%d %H:%M")
                ),
                (Some(_), Ok(SyncOutcome::LeadTimeInPast(_))) => format!(
                    "That lead time already passed for bill <b>#{}</b>, so nothing was scheduled.",
                    bill.id
                ),
                (Some(_), _) => format!(
                    "Saved, but no reminder was scheduled for bill <b>#{}</b> — it needs a room to deliver to.",
                    bill.id
                ),
            })
        }
        Err(_) => Some(format!("Bill #{} not found or access denied.", id)),
    }
}
