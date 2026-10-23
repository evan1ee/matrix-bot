//! `!budget` chat commands: parsing and formatting only. Every rule lives in
//! `FinanceService`; this module translates text to service calls and back.

use crate::application::{AppError, AppServices};
use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::{BudgetPatch, BudgetState, BudgetStatus, Month, NewBudget};
use crate::domain::finance::money::{
    exponent_for, format_amount, looks_like_currency, normalise_currency, parse_amount,
};
use crate::interface::chat::error_reply;
use crate::interface::chat::knowledge::CHAT_INPUT_MAX;
use crate::utils::html_escape;

const USAGE: &str = "⚠️ Usage: <code>!budget [YYYY-MM]</code>, \
<code>!budget set &lt;amount&gt; [CUR] [category]</code>, \
<code>!budget rm [category] [CUR]</code>, \
<code>!budget alerts &lt;on|off&gt; [category] [CUR]</code>";

#[derive(Debug, PartialEq, Eq)]
struct SetArgs {
    limit_minor: i64,
    currency: String,
    category: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
struct ScopeArgs {
    category: Option<String>,
    currency: Option<String>,
}

/// `<amount> [CUR] [category...]`; the amount is read with the currency's exponent.
fn parse_set(input: &str, default_currency: &str) -> Result<SetArgs, String> {
    let mut parts = input.split_whitespace().peekable();
    let amount = parts.next().ok_or("Please provide an amount")?;
    let mut currency = default_currency.to_string();
    if let Some(&next) = parts.peek() {
        if looks_like_currency(next) {
            currency = next.to_uppercase();
            parts.next();
        }
    }
    let currency = normalise_currency(&currency).map_err(|e| e.to_string())?;
    let limit_minor = parse_amount(amount, exponent_for(&currency)).map_err(|e| e.to_string())?;
    if limit_minor <= 0 {
        return Err("Budget limit must be greater than zero".to_string());
    }
    let category = parts.collect::<Vec<_>>().join(" ");
    Ok(SetArgs {
        limit_minor,
        currency,
        category: (!category.is_empty()).then_some(category),
    })
}

/// `[category...] [CUR]`: a trailing currency-looking token is the currency.
fn parse_scope(input: &str) -> ScopeArgs {
    let mut tokens: Vec<&str> = input.split_whitespace().collect();
    let currency = match tokens.last() {
        Some(last) if looks_like_currency(last) => tokens.pop().map(str::to_uppercase),
        _ => None,
    };
    let category = tokens.join(" ");
    ScopeArgs {
        category: (!category.is_empty()).then_some(category),
        currency,
    }
}

fn marker(state: BudgetState) -> &'static str {
    match state {
        BudgetState::Ok => "",
        BudgetState::Warning => "  ⚠",
        BudgetState::Exceeded => "  🚫",
    }
}

fn money(minor: i64, exponent: i64) -> String {
    format_amount(minor, exponent as u32)
}

fn status_line(s: &BudgetStatus) -> String {
    let b = &s.budget;
    let mut line = format!(
        "• <b>{}</b>  {} / {} {}  {} %{}",
        html_escape(b.scope.label()),
        money(s.spent_minor, b.currency_exponent),
        money(b.limit_minor, b.currency_exponent),
        b.currency,
        s.percent_used,
        marker(s.state)
    );
    if let Some(p) = &s.pace {
        line.push_str(&format!(
            " — expected {}, projected {}",
            money(p.expected_minor, b.currency_exponent),
            money(p.projected_minor, b.currency_exponent)
        ));
    }
    if !b.alerts_enabled {
        line.push_str(" (alerts off)");
    }
    line
}

/// One line per budget that is not Ok, for appending to an expense reply.
pub fn budget_lines(statuses: &[BudgetStatus]) -> String {
    statuses
        .iter()
        .filter(|s| s.state != BudgetState::Ok)
        .map(|s| {
            let b = &s.budget;
            format!(
                "{} budget: {} % of {} {} used",
                html_escape(b.scope.label()),
                s.percent_used,
                money(b.limit_minor, b.currency_exponent),
                b.currency
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn fail(err: AppError) -> String {
    error_reply(err).html_body().to_string()
}

pub async fn handle_budget(
    services: &AppServices,
    owner: &AccountId,
    rest: &str,
    default_currency: &str,
) -> String {
    let rest = rest.trim();
    if rest.chars().count() > CHAT_INPUT_MAX {
        return format!("⚠️ Input too long: keep it under {CHAT_INPUT_MAX} characters.");
    }
    let (verb, args) = match rest.split_once(char::is_whitespace) {
        Some((v, a)) => (v.to_lowercase(), a.trim()),
        None => (rest.to_lowercase(), ""),
    };
    match verb.as_str() {
        "" => status(services, owner, None).await,
        "set" => set(services, owner, args, default_currency).await,
        "rm" | "remove" | "delete" => remove(services, owner, args).await,
        "alerts" => alerts(services, owner, args).await,
        _ if args.is_empty() => match Month::parse(rest) {
            Ok(m) => status(services, owner, Some(m)).await,
            Err(_) => USAGE.to_string(),
        },
        _ => USAGE.to_string(),
    }
}

async fn status(services: &AppServices, owner: &AccountId, month: Option<Month>) -> String {
    match services.finance.budget_status(owner, month).await {
        Ok(list) if list.is_empty() => "📊 No budgets yet. Create one with \
<code>!budget set &lt;amount&gt; [CUR] [category]</code>."
            .to_string(),
        Ok(list) => {
            let title = list[0].month;
            let lines: Vec<String> = list.iter().map(status_line).collect();
            format!("📊 <b>Budgets for {title}:</b>\n{}", lines.join("\n"))
        }
        Err(e) => fail(e),
    }
}

async fn set(services: &AppServices, owner: &AccountId, args: &str, default: &str) -> String {
    let parsed = match parse_set(args, default) {
        Ok(p) => p,
        Err(msg) => return format!("⚠️ {}", html_escape(&msg)),
    };
    let new = match NewBudget::new(
        parsed.category.as_deref(),
        &parsed.currency,
        parsed.limit_minor,
        None, // alerts change only through `!budget alerts`
    ) {
        Ok(n) => n,
        Err(e) => return fail(e.into()),
    };
    match services.finance.set_budget(owner, new).await {
        Ok((b, created)) => format!(
            "✅ Budget {} for <b>{}</b>: {} {} per month.",
            if created { "set" } else { "updated" },
            html_escape(b.scope.label()),
            money(b.limit_minor, b.currency_exponent),
            b.currency
        ),
        Err(AppError::Validation(msg)) if msg.contains("No category named") => {
            let name = html_escape(parsed.category.as_deref().unwrap_or_default());
            format!(
                "⚠️ No category named <b>{name}</b>. Create it first with <code>!category add {name}</code>."
            )
        }
        Err(e) => fail(e),
    }
}

async fn remove(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let scope = parse_scope(args);
    let budget = match services
        .finance
        .resolve_budget(owner, scope.category.as_deref(), scope.currency.as_deref())
        .await
    {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    match services.finance.delete_budget(owner, budget.id).await {
        Ok(()) => format!(
            "🗑️ Removed the <b>{}</b> budget ({} {}).",
            html_escape(budget.scope.label()),
            money(budget.limit_minor, budget.currency_exponent),
            budget.currency
        ),
        Err(e) => fail(e),
    }
}

async fn alerts(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let (switch, rest) = args.split_once(char::is_whitespace).unwrap_or((args, ""));
    let enabled = match switch.to_lowercase().as_str() {
        "on" => true,
        "off" => false,
        _ => return USAGE.to_string(),
    };
    let scope = parse_scope(rest);
    let budget = match services
        .finance
        .resolve_budget(owner, scope.category.as_deref(), scope.currency.as_deref())
        .await
    {
        Ok(b) => b,
        Err(e) => return fail(e),
    };
    let patch = BudgetPatch {
        limit_minor: None,
        alerts_enabled: Some(enabled),
    };
    match services
        .finance
        .update_budget(owner, budget.id, patch)
        .await
    {
        Ok(b) => format!(
            "🔔 Alerts for the <b>{}</b> budget are {}.",
            html_escape(b.scope.label()),
            if enabled { "on" } else { "off" }
        ),
        Err(e) => fail(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::finance::Direction;
    use crate::domain::finance::NewTransaction;

    async fn setup() -> (AppServices, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:a"]).await;
        (AppServices::new(&pool), AccountId::from_seed("@alice:a"))
    }

    async fn spend(services: &AppServices, owner: &AccountId, minor: i64, cat: Option<&str>) {
        services
            .finance
            .create_transaction(
                owner,
                NewTransaction {
                    direction: Direction::Expense,
                    amount_minor: minor,
                    currency: "EUR",
                    currency_exponent: 2,
                    category: cat,
                    note: None,
                    receipt_storage_key: None,
                    room_id: "!r",
                    sender_name: "t",
                    occurred_at: chrono::Utc::now(),
                    source: None,
                    create_missing_category: false,
                },
            )
            .await
            .unwrap();
    }

    #[test]
    fn parse_budget_set_table() {
        let ok = parse_set("300", "USD").unwrap();
        assert_eq!(
            (ok.limit_minor, ok.currency.as_str(), ok.category),
            (30000, "USD", None)
        );
        let ok = parse_set("300 EUR food", "USD").unwrap();
        assert_eq!(
            (ok.currency.as_str(), ok.category.as_deref()),
            ("EUR", Some("food"))
        );
        let ok = parse_set("300 food and drink", "USD").unwrap();
        assert_eq!(
            (ok.currency.as_str(), ok.category.as_deref()),
            ("USD", Some("food and drink"))
        );
        let ok = parse_set("5000 JPY", "USD").unwrap();
        assert_eq!(ok.limit_minor, 5000, "JPY has no decimals");
        assert!(parse_set("abc", "USD").is_err());
        assert!(parse_set("0", "USD").is_err());
        assert!(parse_set("", "USD").is_err());
    }

    #[test]
    fn parse_scope_splits_trailing_currency() {
        assert_eq!(
            parse_scope("food EUR"),
            ScopeArgs {
                category: Some("food".into()),
                currency: Some("EUR".into())
            }
        );
        assert_eq!(
            parse_scope("EUR"),
            ScopeArgs {
                category: None,
                currency: Some("EUR".into())
            }
        );
        assert_eq!(
            parse_scope("food and drink"),
            ScopeArgs {
                category: Some("food and drink".into()),
                currency: None
            }
        );
        assert_eq!(
            parse_scope(""),
            ScopeArgs {
                category: None,
                currency: None
            }
        );
    }

    #[tokio::test]
    async fn budget_set_resolves_category_case_insensitively() {
        let (s, o) = setup().await;
        s.finance.create_category(&o, "Food").await.unwrap();
        let reply = handle_budget(&s, &o, "set 300 food", "EUR").await;
        assert!(reply.contains("Food"), "{reply}");
        assert_eq!(s.finance.list_categories(&o).await.unwrap().len(), 1);
        let budgets = s.finance.list_budgets(&o).await.unwrap();
        assert_eq!(budgets.len(), 1);
        assert_eq!(budgets[0].scope.label(), "Food");
    }

    #[tokio::test]
    async fn budget_set_unknown_category_points_to_category_add() {
        let (s, o) = setup().await;
        let reply = handle_budget(&s, &o, "set 300 <b>nope", "EUR").await;
        assert!(reply.contains("!category add"), "{reply}");
        assert!(!reply.contains("<b>nope"), "{reply}");
        assert!(s.finance.list_categories(&o).await.unwrap().is_empty());
        assert!(s.finance.list_budgets(&o).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn budget_set_uses_currency_exponent() {
        let (s, o) = setup().await;
        handle_budget(&s, &o, "set 5000 JPY", "EUR").await;
        let b = &s.finance.list_budgets(&o).await.unwrap()[0];
        assert_eq!((b.limit_minor, b.currency.as_str()), (5000, "JPY"));
    }

    #[tokio::test]
    async fn budget_status_reply_lists_budgets() {
        let (s, o) = setup().await;
        s.finance.create_category(&o, "Food").await.unwrap();
        handle_budget(&s, &o, "set 300 EUR Food", "EUR").await;
        spend(&s, &o, 24600, Some("Food")).await;
        let reply = handle_budget(&s, &o, "", "EUR").await;
        assert!(reply.contains("Food"), "{reply}");
        assert!(reply.contains("246.00 / 300.00"), "{reply}");
        assert!(reply.contains("82 %"), "{reply}");
        assert!(reply.contains('⚠'), "{reply}");
        assert!(
            reply.contains("projected"),
            "current month shows pace: {reply}"
        );
        let past = handle_budget(&s, &o, "2020-01", "EUR").await;
        assert!(past.contains("0.00 / 300.00"), "{past}");
        assert!(!past.contains("projected"), "{past}");
        assert!(handle_budget(&s, &o, "bogus", "EUR")
            .await
            .contains("Usage"));
    }

    #[tokio::test]
    async fn budget_rm_requires_currency_when_ambiguous() {
        let (s, o) = setup().await;
        handle_budget(&s, &o, "set 300 EUR", "EUR").await;
        handle_budget(&s, &o, "set 400 USD", "EUR").await;
        let reply = handle_budget(&s, &o, "rm", "EUR").await;
        assert!(reply.contains("currency"), "{reply}");
        assert_eq!(s.finance.list_budgets(&o).await.unwrap().len(), 2);
        let reply = handle_budget(&s, &o, "rm USD", "EUR").await;
        assert!(reply.contains("Removed"), "{reply}");
        assert_eq!(s.finance.list_budgets(&o).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn budget_set_preserves_alerts_off() {
        let (s, o) = setup().await;
        handle_budget(&s, &o, "set 300 EUR", "EUR").await;
        assert!(
            s.finance.list_budgets(&o).await.unwrap()[0].alerts_enabled,
            "a new budget starts with alerts on"
        );
        handle_budget(&s, &o, "alerts off", "EUR").await;
        let reply = handle_budget(&s, &o, "set 400 EUR", "EUR").await;
        assert!(reply.contains("updated"), "{reply}");
        let b = &s.finance.list_budgets(&o).await.unwrap()[0];
        assert_eq!(b.limit_minor, 40000);
        assert!(!b.alerts_enabled, "set must not re-enable alerts");
    }

    #[tokio::test]
    async fn hostile_category_names_are_escaped_in_status_and_rm() {
        let (s, o) = setup().await;
        s.finance.create_category(&o, "<b>x</b>").await.unwrap();
        handle_budget(&s, &o, "set 300 EUR <b>x</b>", "EUR").await;
        let status = handle_budget(&s, &o, "", "EUR").await;
        assert!(status.contains("&lt;b&gt;x&lt;/b&gt;"), "{status}");
        assert!(!status.contains("<b>x</b>"), "{status}");
        let rm = handle_budget(&s, &o, "rm <b>x</b> EUR", "EUR").await;
        assert!(rm.contains("&lt;b&gt;x&lt;/b&gt;"), "{rm}");
        assert!(!rm.contains("<b>x</b>"), "{rm}");
    }

    #[tokio::test]
    async fn budget_alerts_off() {
        let (s, o) = setup().await;
        handle_budget(&s, &o, "set 300 EUR", "EUR").await;
        let reply = handle_budget(&s, &o, "alerts off", "EUR").await;
        assert!(reply.contains("off"), "{reply}");
        assert!(!s.finance.list_budgets(&o).await.unwrap()[0].alerts_enabled);
        handle_budget(&s, &o, "alerts on", "EUR").await;
        assert!(s.finance.list_budgets(&o).await.unwrap()[0].alerts_enabled);
        assert!(handle_budget(&s, &o, "alerts maybe", "EUR")
            .await
            .contains("Usage"));
    }

    #[tokio::test]
    async fn overlong_input_gets_the_too_long_reply() {
        let (s, o) = setup().await;
        let reply = handle_budget(&s, &o, &format!("set 300 {}", "a".repeat(501)), "EUR").await;
        assert!(reply.contains("too long"), "{reply}");
        assert!(s.finance.list_budgets(&o).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn expense_add_appends_budget_line_at_80_percent() {
        let (s, o) = setup().await;
        s.finance.create_category(&o, "Food").await.unwrap();
        handle_budget(&s, &o, "set 300 EUR Food", "EUR").await;
        let reply = super::super::handler::handle_finance_command(
            &s,
            "!r",
            &o,
            "t",
            "!expense add 255 EUR groceries -c food",
        )
        .await
        .unwrap();
        assert!(reply.contains("Logged"), "{reply}");
        assert!(
            reply.contains("Food budget: 85 % of 300.00 EUR used"),
            "{reply}"
        );
        let quiet = super::super::handler::handle_finance_command(
            &s,
            "!r",
            &o,
            "t",
            "!expense add 1 EUR tea",
        )
        .await
        .unwrap();
        assert!(!quiet.contains("budget"), "{quiet}");
    }

    #[test]
    fn budget_lines_of_nothing_is_empty() {
        assert_eq!(budget_lines(&[]), "");
    }
}
