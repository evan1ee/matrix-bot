use crate::db::DbPool;
use chrono::{Datelike, TimeZone, Utc};
use tracing::error;

use crate::events::EventBroadcaster;
use crate::modules::finance::models::{Direction, Transaction};
use crate::modules::finance::money::{
    exponent_for, format_amount, looks_like_currency, normalise_currency, parse_amount,
};
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::finance::service::{self, FinanceError};
use crate::modules::settings::SettingsRepository;
use crate::utils::html_escape;

// `MAX_NOTE_LEN` and `MAX_CATEGORY_LEN` live in `service` now — this module
// owns the reply wording, not the limits.
const PAGE_SIZE: i64 = 25;

/// The parsed pieces of an `add` command.
struct ParsedEntry {
    amount_minor: i64,
    currency: String,
    exponent: u32,
    note: String,
    category: Option<String>,
}

pub async fn handle_finance_command(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    room_id: &str,
    owner_id: &str,
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
        return handle_category(pool, broadcaster, owner_id, rest).await;
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
            .and_then(|p| p.parse().ok())
            .unwrap_or(1)
            .max(1);
        return handle_list(pool, owner_id, sender_name, page).await;
    }
    if rest_lower.starts_with("summary") {
        return handle_summary(pool, owner_id, sender_name).await;
    }
    if rest_lower.starts_with("delete")
        || rest_lower.starts_with("remove")
        || rest_lower.starts_with("rm")
    {
        let id_str = rest.split_whitespace().nth(1).unwrap_or("");
        return handle_delete(pool, broadcaster, owner_id, id_str).await;
    }
    if rest_lower.starts_with("add") {
        let args = rest[3..].trim();
        // The user's saved currency, so `!expense add 9.99 coffee` books in the
        // currency they actually spend in rather than a hardcoded USD.
        let default_currency = SettingsRepository::get_or_default(pool, owner_id)
            .await
            .currency;
        return handle_add(
            pool,
            broadcaster,
            room_id,
            owner_id,
            sender_name,
            direction,
            args,
            &default_currency,
        )
        .await;
    }

    Some(format!(
        "⚠️ Unknown command. Try <code>!{} add &lt;amount&gt; [CUR] &lt;note&gt; [-c &lt;category&gt;]</code>, \
         <code>!{} list</code>, or <code>!expense summary</code>.",
        keyword, keyword
    ))
}

/// Matches `!<name>` or a bare `<name>`, returning the remainder.
fn strip_command<'a>(lower: &str, body: &'a str, name: &str) -> Option<&'a str> {
    for prefix in [format!("!{}", name), name.to_string()] {
        if lower == prefix {
            return Some("");
        }
        if let Some(stripped) = lower.strip_prefix(&format!("{} ", prefix)) {
            let offset = body.len() - stripped.len();
            return Some(body[offset..].trim());
        }
    }
    None
}

/// Parses `<amount> [CUR] <note> [-c <category>]`.
///
/// The currency is optional and positional: a bare three-letter word directly
/// after the amount is read as a currency code, anything else is the start of
/// the note. When absent, `default_currency` — the sender's saved preference —
/// stands in.
fn parse_entry(args: &str, default_currency: &str) -> Result<ParsedEntry, String> {
    let (body, category) = match args.split_once("-c ") {
        Some((before, after)) => (before.trim(), Some(after.trim().to_string())),
        None => (args.trim(), None),
    };

    let mut parts = body.split_whitespace();
    let amount_raw = parts.next().ok_or("Amount is required")?;

    let remainder: Vec<&str> = parts.collect();
    let (currency_raw, note_parts) = match remainder.first() {
        Some(first) if looks_like_currency(first) => (first.to_uppercase(), &remainder[1..]),
        _ => (default_currency.to_string(), &remainder[..]),
    };

    let currency = normalise_currency(&currency_raw).map_err(|e| e.to_string())?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(amount_raw, exponent).map_err(|e| e.to_string())?;

    // Note length is a business rule, not a parsing concern — `service`
    // enforces it now, on every transport alike.
    let note = note_parts.join(" ");

    Ok(ParsedEntry {
        amount_minor,
        currency,
        exponent,
        note,
        category,
    })
}

async fn handle_add(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    room_id: &str,
    owner_id: &str,
    sender_name: &str,
    direction: Direction,
    args: &str,
    default_currency: &str,
) -> Option<String> {
    let entry = match parse_entry(args, default_currency) {
        Ok(entry) => entry,
        Err(err) => {
            return Some(format!(
                "⚠️ {}.<br/>Usage: <code>!expense add 15.50 EUR lunch -c food</code>",
                html_escape(&err)
            ))
        }
    };

    let note = if entry.note.is_empty() {
        None
    } else {
        Some(entry.note.as_str())
    };

    match service::create_transaction(
        pool,
        broadcaster,
        owner_id,
        service::NewTransaction {
            direction: direction.as_str(),
            amount_minor: entry.amount_minor,
            currency: &entry.currency,
            currency_exponent: entry.exponent as i64,
            category: entry.category.as_deref(),
            note,
            room_id,
            sender_name,
            occurred_at: Utc::now(),
            source: None,
            // A named category must already exist. Creating one implicitly
            // would let a typo silently fragment a category into two.
            create_missing_category: false,
        },
    )
    .await
    {
        Ok(tx) => Some(format!(
            "{} Logged <b>{} {}</b>{}{} <i>(#{})</i>",
            if direction == Direction::Expense {
                "💸"
            } else {
                "💰"
            },
            html_escape(&format_amount(tx.amount_minor, entry.exponent)),
            html_escape(&tx.currency),
            tx.note
                .as_deref()
                .map(|n| format!(" — {}", html_escape(n)))
                .unwrap_or_default(),
            entry
                .category
                .as_deref()
                .map(|c| format!(" [{}]", html_escape(c)))
                .unwrap_or_default(),
            tx.id
        )),
        Err(FinanceError::NoteTooLong { max }) => Some(format!(
            "⚠️ Note is too long (max {} characters).<br/>Usage: <code>!expense add 15.50 EUR lunch -c food</code>",
            max
        )),
        Err(FinanceError::CategoryNameInvalid { max }) => {
            Some(format!("⚠️ A category name must be 1–{} characters.", max))
        }
        Err(FinanceError::UnknownCategory { name }) => Some(format!(
            "⚠️ No category named <b>{}</b>. Create it with <code>!category add {}</code>.",
            html_escape(&name),
            html_escape(&name)
        )),
        Err(FinanceError::Db(err)) => {
            error!("Failed to record transaction: {:?}", err);
            Some("❌ Database error recording the entry.".to_string())
        }
        Err(_) => Some("❌ Database error recording the entry.".to_string()),
    }
}

async fn handle_list(
    pool: &DbPool,
    owner_id: &str,
    sender_name: &str,
    page: i64,
) -> Option<String> {
    let offset = (page - 1) * PAGE_SIZE;
    match FinanceRepository::list(pool, owner_id, None, PAGE_SIZE, offset).await {
        Ok(items) if items.is_empty() => Some(format!(
            "📒 No ledger entries for {} on page {}.",
            html_escape(sender_name),
            page
        )),
        Ok(items) => Some(format_ledger(&items, sender_name, page)),
        Err(err) => {
            error!("Failed to list transactions: {:?}", err);
            Some("❌ Database error loading the ledger.".to_string())
        }
    }
}

fn format_ledger(items: &[Transaction], sender_name: &str, page: i64) -> String {
    let mut html = format!(
        "📒 <b>Ledger for {} (Page {}):</b><br/>",
        html_escape(sender_name),
        page
    );

    for tx in items {
        let is_expense = tx.direction == "expense";
        html.push_str(&format!(
            "• [<b>#{}</b>] {} <b>{} {}</b>{}{}<br/>",
            tx.id,
            if is_expense { "💸" } else { "💰" },
            html_escape(&format_amount(tx.amount_minor, tx.currency_exponent as u32)),
            html_escape(&tx.currency),
            tx.note
                .as_deref()
                .map(|n| format!(" — {}", html_escape(n)))
                .unwrap_or_default(),
            tx.category_name
                .as_deref()
                .map(|c| format!(" <i>[{}]</i>", html_escape(c)))
                .unwrap_or_default(),
        ));
    }

    html.push_str("<br/><i>(Use <code>!expense delete &lt;id&gt;</code> to remove one)</i>");
    html
}

async fn handle_summary(pool: &DbPool, owner_id: &str, sender_name: &str) -> Option<String> {
    let now = Utc::now();
    let start = Utc
        .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .unwrap_or(now);
    let end = if now.month() == 12 {
        Utc.with_ymd_and_hms(now.year() + 1, 1, 1, 0, 0, 0)
            .single()
            .unwrap_or(now)
    } else {
        Utc.with_ymd_and_hms(now.year(), now.month() + 1, 1, 0, 0, 0)
            .single()
            .unwrap_or(now)
    };

    match FinanceRepository::summary(pool, owner_id, start, end).await {
        Ok(summaries) if summaries.is_empty() => {
            Some("📊 Nothing recorded this month.".to_string())
        }
        Ok(summaries) => {
            let mut html = format!(
                "📊 <b>{} — {}:</b><br/>",
                html_escape(sender_name),
                start.format("%B %Y")
            );

            for summary in summaries {
                let exp = summary.currency_exponent as u32;
                html.push_str(&format!(
                    "<br/><b>{}</b> — in {} / out {} / net {}<br/>",
                    html_escape(&summary.currency),
                    html_escape(&format_amount(summary.income_minor, exp)),
                    html_escape(&format_amount(summary.expense_minor, exp)),
                    html_escape(&format_amount(summary.net_minor, exp)),
                ));
                for bucket in summary.categories.iter().filter(|c| c.expense_minor > 0) {
                    html.push_str(&format!(
                        "&nbsp;&nbsp;• {}: {}<br/>",
                        html_escape(bucket.category.as_deref().unwrap_or("Uncategorised")),
                        html_escape(&format_amount(bucket.expense_minor, exp)),
                    ));
                }
            }

            Some(html)
        }
        Err(err) => {
            error!("Failed to summarise transactions: {:?}", err);
            Some("❌ Database error building the summary.".to_string())
        }
    }
}

async fn handle_delete(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    id_str: &str,
) -> Option<String> {
    let Ok(id) = id_str.trim().parse::<i64>() else {
        return Some("⚠️ Usage: <code>!expense delete &lt;id&gt;</code>".to_string());
    };

    // Ownership is checked before deleting; the id alone is not authority.
    match service::delete_transaction(pool, broadcaster, owner_id, id).await {
        Ok(_) => Some(format!("🗑️ Removed entry <b>#{}</b>.", id)),
        Err(FinanceError::TransactionNotFound) => {
            Some(format!("⚠️ Entry <b>#{}</b> not found.", id))
        }
        Err(FinanceError::Db(err)) => {
            error!("Failed to delete transaction: {:?}", err);
            Some("❌ Database error removing the entry.".to_string())
        }
        Err(_) => Some("❌ Database error removing the entry.".to_string()),
    }
}

async fn handle_category(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    rest: &str,
) -> Option<String> {
    let lower = rest.to_lowercase();

    if lower.is_empty() || lower.starts_with("list") {
        return match FinanceRepository::list_categories(pool, owner_id).await {
            Ok(cats) if cats.is_empty() => Some(
                "🏷️ No categories yet. Add one with <code>!category add food</code>.".to_string(),
            ),
            Ok(cats) => {
                let names: Vec<String> = cats
                    .iter()
                    .map(|c| format!("<code>{}</code>", html_escape(&c.name)))
                    .collect();
                Some(format!("🏷️ <b>Your categories:</b> {}", names.join(", ")))
            }
            Err(err) => {
                error!("Failed to list categories: {:?}", err);
                Some("❌ Database error loading categories.".to_string())
            }
        };
    }

    if let Some(name) = lower.strip_prefix("add ") {
        let name = rest[rest.len() - name.len()..].trim();
        return match service::create_category(pool, broadcaster, owner_id, name).await {
            Ok(cat) => Some(format!(
                "🏷️ Added category <b>{}</b>.",
                html_escape(&cat.name)
            )),
            Err(FinanceError::CategoryNameInvalid { max }) => {
                Some(format!("⚠️ A category name must be 1–{} characters.", max))
            }
            Err(FinanceError::CategoryExists) => Some(format!(
                "⚠️ You already have a category named <b>{}</b>.",
                html_escape(name)
            )),
            Err(FinanceError::Db(err)) => {
                error!("Failed to create category: {:?}", err);
                Some("❌ Database error adding the category.".to_string())
            }
            Err(_) => Some("❌ Database error adding the category.".to_string()),
        };
    }

    if let Some(name) = lower
        .strip_prefix("remove ")
        .or_else(|| lower.strip_prefix("delete "))
    {
        let name = rest[rest.len() - name.len()..].trim();
        return match FinanceRepository::find_category_by_name(pool, owner_id, name).await {
            Ok(Some(cat)) => {
                match service::delete_category(pool, broadcaster, owner_id, cat.id).await {
                    Ok(orphaned) => Some(format!(
                        "🏷️ Removed <b>{}</b>. {} entr{} kept and left uncategorised.",
                        html_escape(&cat.name),
                        orphaned,
                        if orphaned == 1 { "y" } else { "ies" }
                    )),
                    Err(FinanceError::CategoryNotFound) => {
                        Some("⚠️ Category not found.".to_string())
                    }
                    Err(FinanceError::Db(err)) => {
                        error!("Failed to delete category: {:?}", err);
                        Some("❌ Database error removing the category.".to_string())
                    }
                    Err(_) => Some("❌ Database error removing the category.".to_string()),
                }
            }
            Ok(None) => Some(format!(
                "⚠️ No category named <b>{}</b>.",
                html_escape(name)
            )),
            Err(err) => {
                error!("Failed to look up category: {:?}", err);
                Some("❌ Database error removing the category.".to_string())
            }
        };
    }

    Some("⚠️ Usage: <code>!category [add|list|remove] &lt;name&gt;</code>".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

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

    /// An ordinary three-letter word must stay in the note. Treating any
    /// three-letter token as a currency silently books "tea" as a currency and
    /// drops the word from the note.
    #[test]
    fn test_three_letter_note_word_is_not_eaten_as_currency() {
        let entry = parse_entry("3.00 tea and cake", "USD").unwrap();
        assert_eq!(entry.currency, "USD");
        assert_eq!(entry.note, "tea and cake");
    }

    /// A real code is still recognised in either case.
    #[test]
    fn test_currency_is_recognised_upper_or_lower() {
        assert_eq!(parse_entry("5 EUR bread", "USD").unwrap().currency, "EUR");
        assert_eq!(parse_entry("5 eur bread", "USD").unwrap().currency, "EUR");
        assert_eq!(parse_entry("5 eur bread", "USD").unwrap().note, "bread");
        // Uppercase is honoured even for an uncommon code.
        assert_eq!(parse_entry("5 XYZ thing", "USD").unwrap().currency, "XYZ");
    }

    #[test]
    fn test_zero_decimal_currency() {
        let entry = parse_entry("500 JPY ramen", "USD").unwrap();
        assert_eq!(entry.amount_minor, 500);
        assert_eq!(entry.exponent, 0);
        assert_eq!(entry.currency, "JPY");
    }

    #[test]
    fn test_rejects_bad_amounts() {
        assert!(parse_entry("abc lunch", "USD").is_err());
        assert!(parse_entry("15.501 EUR lunch", "USD").is_err());
        assert!(parse_entry("-5 EUR lunch", "USD").is_err());
        assert!(parse_entry("", "USD").is_err());
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

    #[tokio::test]
    async fn test_finance_alias_and_rm() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db::schema::run_migrations(&pool).await.unwrap();

        // Add expense via !expense add
        let add_res = handle_finance_command(
            &pool,
            None,
            "!room:1",
            "@alice:1",
            "alice",
            "!expense add 12.50 USD lunch",
        )
        .await
        .unwrap();
        assert!(add_res.contains("Logged"));

        // Query via !finance
        let fin_res =
            handle_finance_command(&pool, None, "!room:1", "@alice:1", "alice", "!finance")
                .await
                .unwrap();
        assert!(fin_res.contains("lunch"));

        // Delete via !expense rm 1
        let del_res =
            handle_finance_command(&pool, None, "!room:1", "@alice:1", "alice", "!expense rm 1")
                .await
                .unwrap();
        assert!(del_res.contains("Removed entry"));
    }

    /// The bug this refactor exists to fix: `!expense add` never reached an
    /// open browser because the Matrix path never broadcast anything.
    #[tokio::test]
    async fn test_a_matrix_add_announces_itself() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db::schema::run_migrations(&pool).await.unwrap();
        let broadcaster = crate::events::EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();

        let reply = handle_finance_command(
            &pool,
            Some(&broadcaster),
            "!room:1",
            "@alice:1",
            "alice",
            "!expense add 12.50 USD lunch",
        )
        .await
        .unwrap();
        assert!(reply.contains("Logged"));

        let event = rx.try_recv().expect("the add must broadcast an event");
        assert!(matches!(
            event,
            crate::events::SystemEvent::TransactionCreated(_)
        ));
    }

    /// The regression this refactor exists to revert: an unknown category
    /// name typed into `!expense add` must be refused, not silently created —
    /// a typo would otherwise fragment a category into two. This is a smoke
    /// test that the Matrix transport renders the refusal in its own format;
    /// `service::test_create_missing_category_flag_controls_the_side_effect`
    /// covers the side-effect-free rejection itself.
    #[tokio::test]
    async fn test_expense_add_refuses_an_unknown_category() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db::schema::run_migrations(&pool).await.unwrap();

        let reply = handle_finance_command(
            &pool,
            None,
            "!room:1",
            "@alice:1",
            "alice",
            "!expense add 12.50 USD lunch -c Groceries",
        )
        .await
        .unwrap();

        assert!(
            reply.contains("No category named <b>Groceries</b>"),
            "unexpected reply: {reply}"
        );
        assert!(reply.contains("!category add Groceries"));
        assert_eq!(
            FinanceRepository::list_categories(&pool, "@alice:1")
                .await
                .unwrap()
                .len(),
            0,
            "an unknown category must not be created as a side effect"
        );
    }
}
