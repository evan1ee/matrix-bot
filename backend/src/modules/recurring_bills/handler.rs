use crate::db::Database;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::CommandContext;
use crate::utils::html::html_escape;

use super::models::CreateBillPayload;
use super::repository::RecurringBillRepository;

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
        _ => Some(format!(
            "Unknown subcommand <code>{}</code>. Usage: <code>!bill [list|add|pay|delete]</code>",
            html_escape(&sub_cmd)
        )),
    }
}

async fn handle_list(ctx: &CommandContext, db: &Database) -> Option<String> {
    let bills = RecurringBillRepository::list_by_owner(&db.pool, &ctx.sender_mxid)
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
    };

    match RecurringBillRepository::create(&db.pool, &ctx.room_id, &ctx.sender_mxid, &ctx.sender_name, payload).await {
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

    match RecurringBillRepository::mark_paid(&db.pool, id, &ctx.sender_mxid).await {
        Ok(bill) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillPaid {
                    id: bill.id,
                    owner_id: bill.sender_mxid.clone(),
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

    match RecurringBillRepository::delete(&db.pool, id, &ctx.sender_mxid).await {
        Ok(true) => {
            if let Some(b) = broadcaster {
                b.send(SystemEvent::RecurringBillDeleted {
                    id,
                    owner_id: ctx.sender_mxid.clone(),
                });
            }
            Some(format!("Deleted bill <b>#{}</b>.", id))
        }
        _ => Some(format!("Bill #{} not found or access denied.", id)),
    }
}
