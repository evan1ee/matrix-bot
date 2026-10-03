use chrono::{Datelike, TimeZone, Utc};
use tracing::error;

use super::category::handle_category;
use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::finance::money::{
    exponent_for, format_amount, looks_like_currency, normalise_currency, parse_amount,
};
use crate::domain::finance::{Direction, ListTransactionsFilter, NewTransaction, Transaction};
use crate::utils::html_escape;

const PAGE_SIZE: i64 = 25;

struct ParsedEntry {
    amount_minor: i64,
    currency: String,
    exponent: u32,
    note: String,
    category: Option<String>,
}

pub async fn handle_finance_command(
    services: &AppServices,
    room_id: &str,
    owner_id: &AccountId,
    sender_name: &str,
    clean_body: &str,
) -> Option<String> {
    let body = clean_body.trim();
    let lower = body.to_lowercase();

    let (keyword, rest) = if let Some(rest) = strip_command(&lower, body, "expense") {
        ("expense", rest)
    } else if let Some(rest) = strip_command(&lower, body, "income") {
        ("income", rest)
    } else if let Some(rest) = strip_command(&lower, body, "finance") {
        ("expense", rest)
    } else {
        let rest = strip_command(&lower, body, "category")?;
        ("category", rest)
    };

    if keyword == "category" {
        return handle_category(&services.finance, owner_id, rest).await;
    }

    let rest_lower = rest.to_lowercase();
    let direction = if keyword == "income" {
        Direction::Income
    } else {
        Direction::Expense
    };

    if rest_lower.is_empty() || rest_lower.starts_with("list") {
        let page = rest
            .split_whitespace()
            .nth(1)
            .and_then(|p| p.parse::<i64>().ok())
            .unwrap_or(1)
            .max(1);
        let offset = (page - 1) * PAGE_SIZE;
        let filter = ListTransactionsFilter {
            direction: Some(direction),
            limit: Some(PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        };
        return match services.finance.list_transactions(owner_id, &filter).await {
            Ok(txs) => Some(format_ledger(&txs, direction)),
            Err(e) => {
                error!("Failed to list transactions: {:?}", e);
                Some("❌ Database error fetching your entries.".to_string())
            }
        };
    }

    if let Some(sub) = rest_lower
        .strip_prefix("rm ")
        .or_else(|| rest_lower.strip_prefix("delete "))
        .or_else(|| rest_lower.strip_prefix("del "))
    {
        let id_str = sub.trim();
        let Ok(id) = id_str.parse::<i64>() else {
            return Some("⚠️ Please provide a valid transaction ID to remove.".to_string());
        };

        return match services.finance.delete_transaction(owner_id, id).await {
            Ok(tx) => Some(format!(
                "🗑️ Removed entry <b>#{}</b>: {} {} ({})",
                tx.id,
                format_amount(tx.amount_minor, tx.currency_exponent as u32),
                tx.currency,
                html_escape(tx.note.as_deref().unwrap_or("no note"))
            )),
            Err(crate::application::AppError::NotFound(_)) => {
                Some(format!("⚠️ Entry <b>#{}</b> not found.", id))
            }
            Err(err) => {
                error!("Failed to delete transaction: {:?}", err);
                Some("❌ Database error removing the entry.".to_string())
            }
        };
    }

    if let Some(sub) = rest_lower.strip_prefix("summary") {
        return handle_summary_command(services, owner_id, sub.trim()).await;
    }

    let entry_str = if let Some(sub) = rest.strip_prefix("add ") {
        sub.trim()
    } else {
        rest
    };

    let user_settings = services.settings.get_or_default(owner_id).await;
    let default_currency = user_settings.currency;

    let parsed = match parse_entry(entry_str, &default_currency) {
        Ok(p) => p,
        Err(msg) => return Some(format!("⚠️ {msg}")),
    };

    let note_opt = if parsed.note.is_empty() {
        None
    } else {
        Some(parsed.note.as_str())
    };

    let input = NewTransaction {
        direction,
        amount_minor: parsed.amount_minor,
        currency: &parsed.currency,
        currency_exponent: parsed.exponent as i64,
        category: parsed.category.as_deref(),
        note: note_opt,
        room_id,
        sender_name,
        occurred_at: Utc::now(),
        source: None,
        create_missing_category: false,
    };

    match services.finance.create_transaction(owner_id, input).await {
        Ok(tx) => {
            let cat_str = tx
                .category_name
                .as_deref()
                .map(|c| format!(" [🏷️ {}]", html_escape(c)))
                .unwrap_or_default();
            let note_str = tx
                .note
                .as_deref()
                .map(|n| format!(" — {}", html_escape(n)))
                .unwrap_or_default();

            Some(format!(
                "✅ Logged {} <b>{} {}</b>{}{}",
                tx.direction.as_str(),
                format_amount(tx.amount_minor, tx.currency_exponent as u32),
                tx.currency,
                cat_str,
                note_str
            ))
        }
        Err(crate::application::AppError::Validation(msg)) => {
            if msg.contains("No category named") {
                let name = parsed.category.as_deref().unwrap_or("unknown");
                Some(format!(
                    "⚠️ No category named <b>{}</b>. Create it first with <code>!category add {}</code>.",
                    html_escape(name),
                    html_escape(name)
                ))
            } else {
                Some(format!("⚠️ {msg}"))
            }
        }
        Err(err) => {
            error!("Failed to record transaction: {:?}", err);
            Some("❌ Database error logging the entry.".to_string())
        }
    }
}

async fn handle_summary_command(
    services: &AppServices,
    owner_id: &AccountId,
    args: &str,
) -> Option<String> {
    let user_settings = services.settings.get_or_default(owner_id).await;
    let tz = user_settings.tz();
    let now = Utc::now().with_timezone(&tz);

    let (from, to, period_name) = if args.starts_with("year") {
        let start = tz.with_ymd_and_hms(now.year(), 1, 1, 0, 0, 0).unwrap();
        let end = tz.with_ymd_and_hms(now.year() + 1, 1, 1, 0, 0, 0).unwrap();
        (
            start.with_timezone(&Utc),
            end.with_timezone(&Utc),
            format!("Year {}", now.year()),
        )
    } else {
        let (next_y, next_m) = if now.month() == 12 {
            (now.year() + 1, 1)
        } else {
            (now.year(), now.month() + 1)
        };
        let start = tz
            .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .unwrap();
        let end = tz.with_ymd_and_hms(next_y, next_m, 1, 0, 0, 0).unwrap();
        (
            start.with_timezone(&Utc),
            end.with_timezone(&Utc),
            now.format("%B %Y").to_string(),
        )
    };

    match services.finance.summary(owner_id, from, to).await {
        Ok(summaries) => Some(format_summary(&summaries, &period_name)),
        Err(e) => {
            error!("Failed to compute finance summary: {:?}", e);
            Some("❌ Database error computing summary.".to_string())
        }
    }
}

fn strip_command<'a>(lower: &str, orig: &'a str, name: &str) -> Option<&'a str> {
    let bang = format!("!{name}");
    if let Some(rest) = lower.strip_prefix(&bang) {
        if rest.is_empty() || rest.starts_with(' ') {
            return Some(orig[bang.len()..].trim());
        }
    }
    if let Some(rest) = lower.strip_prefix(name) {
        if rest.is_empty() || rest.starts_with(' ') {
            return Some(orig[name.len()..].trim());
        }
    }
    None
}

fn parse_entry(input: &str, default_currency: &str) -> Result<ParsedEntry, String> {
    let mut parts = input.split_whitespace().peekable();
    let amount_str = parts.next().ok_or("Please provide an amount")?;

    let mut currency = default_currency.to_string();
    if let Some(&next) = parts.peek() {
        if looks_like_currency(next) {
            currency = next.to_uppercase();
            parts.next();
        }
    }

    let remainder: Vec<&str> = parts.collect();
    let full_note = remainder.join(" ");

    let (note, category) = if let Some(idx) = full_note.find(" -c ") {
        let n = full_note[..idx].trim().to_string();
        let c = full_note[idx + 4..].trim().to_string();
        (n, if c.is_empty() { None } else { Some(c) })
    } else {
        (full_note, None)
    };

    let norm_curr = normalise_currency(&currency).map_err(|e| e.to_string())?;
    let exp = exponent_for(&norm_curr);
    let minor = parse_amount(amount_str, exp).map_err(|e| e.to_string())?;

    Ok(ParsedEntry {
        amount_minor: minor,
        currency: norm_curr,
        exponent: exp,
        note,
        category,
    })
}

fn format_ledger(txs: &[Transaction], direction: Direction) -> String {
    if txs.is_empty() {
        return format!("No {} entries recorded yet.", direction.as_str());
    }

    let rows: Vec<String> = txs
        .iter()
        .map(|t| {
            let cat = t
                .category_name
                .as_deref()
                .map(|c| format!(" [🏷️ {}]", html_escape(c)))
                .unwrap_or_default();
            let note = t
                .note
                .as_deref()
                .map(|n| format!(" — {}", html_escape(n)))
                .unwrap_or_default();
            format!(
                "• <code>#{}</code> <b>{} {}</b>{}{}",
                t.id,
                format_amount(t.amount_minor, t.currency_exponent as u32),
                t.currency,
                cat,
                note
            )
        })
        .collect();

    format!(
        "<b>Recent {}s:</b>\n{}",
        direction.as_str(),
        rows.join("\n")
    )
}

fn format_summary(
    summaries: &[crate::domain::finance::CurrencySummary],
    period_name: &str,
) -> String {
    if summaries.is_empty() {
        return format!("📊 <b>Summary for {period_name}:</b> No transactions found.");
    }

    let mut out = format!("📊 <b>Summary for {period_name}:</b>\n");
    for s in summaries {
        out.push_str(&format!(
            "\n<b>{}</b> (Expense: {}, Income: {}, Net: {})\n",
            s.currency,
            format_amount(s.expense_minor, s.currency_exponent as u32),
            format_amount(s.income_minor, s.currency_exponent as u32),
            format_amount(s.net_minor, s.currency_exponent as u32)
        ));
        for c in &s.categories {
            let cat_name = c.category.as_deref().unwrap_or("Uncategorized");
            out.push_str(&format!(
                "  • {}: {}\n",
                html_escape(cat_name),
                format_amount(c.expense_minor, s.currency_exponent as u32)
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_amount_currency_note_and_category() {
        let entry = parse_entry("15.50 EUR lunch with team -c food", "USD").unwrap();
        assert_eq!(entry.amount_minor, 1550);
        assert_eq!(entry.currency, "EUR");
        assert_eq!(entry.note, "lunch with team");
        assert_eq!(entry.category.as_deref(), Some("food"));
    }

    #[test]
    fn test_currency_is_optional() {
        let entry = parse_entry("9.99 coffee", "USD").unwrap();
        assert_eq!(entry.amount_minor, 999);
        assert_eq!(entry.currency, "USD");
        assert_eq!(entry.note, "coffee");
        assert!(entry.category.is_none());
    }

    #[test]
    fn test_command_stripping() {
        assert_eq!(
            strip_command("!expense add 5", "!expense add 5", "expense"),
            Some("add 5")
        );
        assert_eq!(
            strip_command("expense list", "expense list", "expense"),
            Some("list")
        );
        assert_eq!(strip_command("!expense", "!expense", "expense"), Some(""));
        assert_eq!(strip_command("!todo add x", "!todo add x", "expense"), None);
    }
}
