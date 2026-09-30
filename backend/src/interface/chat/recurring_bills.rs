//! Matrix chat module for recurring bills and subscriptions.

use async_trait::async_trait;

use crate::application::recurring_bills::RecurringBillsService;
use crate::application::AppServices;
use crate::db::Database;
use crate::domain::recurring_bills::{NewRecurringBill, RecurringBillPatch};
use crate::domain::reminders::{SyncOutcome, MAX_LEAD_MINUTES};
use crate::interface::chat::{BotModule, CommandContext, CommandHelp};
use crate::utils::html_escape;

pub struct RecurringBillsModule;

impl RecurringBillsModule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RecurringBillsModule {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl BotModule for RecurringBillsModule {
    fn name(&self) -> &str {
        "Recurring Bills"
    }

    fn commands(&self) -> Vec<CommandHelp> {
        vec![
            CommandHelp {
                name: "bill list".to_string(),
                usage: "!bill list".to_string(),
                description: "List all recurring bills, subscriptions, installments, and transfers"
                    .to_string(),
            },
            CommandHelp {
                name: "bill add".to_string(),
                usage: "!bill add <name> <amount> <currency> <category> [frequency]".to_string(),
                description: "Add a new recurring bill or subscription".to_string(),
            },
            CommandHelp {
                name: "bill pay".to_string(),
                usage: "!bill pay <id>".to_string(),
                description: "Mark a recurring bill as paid for the current period".to_string(),
            },
            CommandHelp {
                name: "bill remind".to_string(),
                usage: "!bill remind <id> <minutes|off>".to_string(),
                description: "Remind me this many minutes before the bill is due".to_string(),
            },
            CommandHelp {
                name: "bill delete".to_string(),
                usage: "!bill delete <id>".to_string(),
                description: "Delete a recurring bill".to_string(),
            },
        ]
    }

    async fn handle_command(
        &self,
        ctx: &CommandContext,
        services: &AppServices,
        _db: &Database,
    ) -> Option<String> {
        if ctx.command == "bill" || ctx.command == "bills" || ctx.command == "recurring" {
            handle_bill_command(ctx, &services.recurring_bills).await
        } else {
            None
        }
    }
}

pub async fn handle_bill_command(
    ctx: &CommandContext,
    bills: &RecurringBillsService,
) -> Option<String> {
    let args: Vec<&str> = ctx.raw_args.split_whitespace().collect();
    let sub_cmd = args.first().copied().unwrap_or("list").to_lowercase();

    match sub_cmd.as_str() {
        "list" | "" => handle_list(ctx, bills).await,
        "add" => handle_add(ctx, bills, &args[1..]).await,
        "pay" => handle_pay(ctx, bills, &args[1..]).await,
        "delete" | "del" | "rm" => handle_delete(ctx, bills, &args[1..]).await,
        "remind" => handle_remind(ctx, bills, &args[1..]).await,
        _ => Some(format!(
            "Unknown subcommand <code>{}</code>. Usage: <code>!bill [list|add|pay|delete]</code>",
            html_escape(&sub_cmd)
        )),
    }
}

async fn handle_list(ctx: &CommandContext, bills: &RecurringBillsService) -> Option<String> {
    let list = bills.list_by_owner(&ctx.owner_id).await.unwrap_or_default();

    if list.is_empty() {
        return Some("No recurring bills recorded. Use <code>!bill add &lt;name&gt; &lt;amount&gt; &lt;currency&gt; &lt;category&gt;</code> to create one.".to_string());
    }

    let mut html = String::from("<b>📋 Recurring Bills & Subscriptions</b><br/><ul>");

    for bill in &list {
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
    bills: &RecurringBillsService,
    args: &[&str],
) -> Option<String> {
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

    let input = NewRecurringBill {
        room_id: ctx.room_id.clone(),
        sender_name: ctx.sender_name.clone(),
        name: name.to_string(),
        category,
        amount_minor,
        currency: currency.clone(),
        currency_exponent: 2,
        frequency,
        due_day: None,
        next_due_at,
        total_installments: None,
        auto_pay: false,
        notes: None,
        remind_before_minutes: None,
        remind_room_id: None,
    };

    match bills.create(&ctx.owner_id, input).await {
        Ok((bill, _)) => Some(format!(
            "Created recurring bill <b>#{}</b>: {} ({} {:.2})",
            bill.id,
            html_escape(&bill.name),
            html_escape(&bill.currency),
            amount
        )),
        Err(err) => Some(format!("Failed to create bill: {}", err)),
    }
}

async fn handle_pay(
    ctx: &CommandContext,
    bills: &RecurringBillsService,
    args: &[&str],
) -> Option<String> {
    if args.is_empty() {
        return Some("Usage: <code>!bill pay &lt;id&gt;</code>".to_string());
    }

    let id: i64 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => return Some("Invalid bill ID.".to_string()),
    };

    match bills.mark_paid(&ctx.owner_id, id).await {
        Ok((bill, _, _)) => Some(format!(
            "Marked bill <b>#{}</b> ({}) as paid. Next due: <b>{}</b>",
            bill.id,
            html_escape(&bill.name),
            html_escape(&bill.next_due_at)
        )),
        Err(_) => Some(format!("Bill #{} not found or access denied.", id)),
    }
}

async fn handle_delete(
    ctx: &CommandContext,
    bills: &RecurringBillsService,
    args: &[&str],
) -> Option<String> {
    if args.is_empty() {
        return Some("Usage: <code>!bill delete &lt;id&gt;</code>".to_string());
    }

    let id: i64 = match args[0].parse() {
        Ok(v) => v,
        Err(_) => return Some("Invalid bill ID.".to_string()),
    };

    match bills.delete(&ctx.owner_id, id).await {
        Ok(_) => Some(format!("Deleted bill <b>#{}</b>.", id)),
        Err(_) => Some(format!("Bill #{} not found or access denied.", id)),
    }
}

async fn handle_remind(
    ctx: &CommandContext,
    bills: &RecurringBillsService,
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
            Ok(v) if v > 0 && v <= MAX_LEAD_MINUTES => Some(v),
            _ => {
                return Some(format!(
                    "Lead time must be between 1 and {} minutes, or <code>off</code>.",
                    MAX_LEAD_MINUTES
                ))
            }
        },
    };

    let patch = RecurringBillPatch {
        remind_before_minutes: Some(lead),
        ..Default::default()
    };

    match bills.update(&ctx.owner_id, id, patch).await {
        Ok((bill, outcome)) => Some(match (lead, outcome) {
            (None, _) => format!("Reminder cleared for bill <b>#{}</b>.", bill.id),
            (Some(m), SyncOutcome::Scheduled(at)) => format!(
                "Will remind {} minutes before bill <b>#{}</b> ({}) is due — {} UTC.",
                m,
                bill.id,
                html_escape(&bill.name),
                at.format("%Y-%m-%d %H:%M")
            ),
            (Some(_), SyncOutcome::LeadTimeInPast(_)) => format!(
                "That lead time already passed for bill <b>#{}</b>, so nothing was scheduled.",
                bill.id
            ),
            (Some(_), _) => format!(
                "Saved, but no reminder was scheduled for bill <b>#{}</b> — it needs a room to deliver to.",
                bill.id
            ),
        }),
        Err(_) => Some(format!("Bill #{} not found or access denied.", id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;

    fn ctx(cmd: &str, raw_args: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:1".to_string(),
            sender_mxid: "@alice:a".to_string(),
            owner_id: AccountId::from_seed("@alice:a"),
            sender_name: "alice".to_string(),
            command: cmd.to_string(),
            raw_args: raw_args.to_string(),
            full_body: format!("!{} {}", cmd, raw_args),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn setup_services() -> (Database, AppServices) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        let services = AppServices::new(&pool);
        (Database { pool }, services)
    }

    #[tokio::test]
    async fn test_recurring_bills_module_lifecycle() {
        let (db, services) = setup_services().await;
        let module = RecurringBillsModule::new();

        // 1. List when empty
        let res = module
            .handle_command(&ctx("bill", "list"), &services, &db)
            .await
            .unwrap();
        assert!(res.contains("No recurring bills recorded"));

        // 2. Add a bill
        let res = module
            .handle_command(
                &ctx("bill", "add Netflix 15.99 EUR subscription monthly"),
                &services,
                &db,
            )
            .await
            .unwrap();
        assert!(res.contains("Created recurring bill <b>#1</b>: Netflix (EUR 15.99)"));

        // 3. List
        let res = module
            .handle_command(&ctx("bill", "list"), &services, &db)
            .await
            .unwrap();
        assert!(res.contains("Netflix"));
        assert!(res.contains("15.99"));

        // 4. Remind
        let res = module
            .handle_command(&ctx("bill", "remind 1 60"), &services, &db)
            .await
            .unwrap();
        assert!(res.contains("bill <b>#1</b>"));

        // 5. Pay
        let res = module
            .handle_command(&ctx("bill", "pay 1"), &services, &db)
            .await
            .unwrap();
        assert!(res.contains("Marked bill <b>#1</b> (Netflix) as paid"));

        // 6. Delete
        let res = module
            .handle_command(&ctx("bill", "delete 1"), &services, &db)
            .await
            .unwrap();
        assert!(res.contains("Deleted bill <b>#1</b>"));
    }
}
