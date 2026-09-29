use tracing::error;

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::finance::money::{format_amount, parse_amount};
use crate::domain::portfolio::models::normalise_symbol;
use crate::interface::chat::portfolio::alerts_handler::handle_alert_command;
use crate::interface::chat::portfolio::cash::handle_cash;
use crate::interface::chat::portfolio::trade_cmd::{handle_buy, handle_record, handle_sell};
use crate::interface::chat::portfolio::view::{handle_list, handle_summary, usage};
use crate::utils::html::html_escape;

pub async fn handle_portfolio_command(
    services: &AppServices,
    owner: &AccountId,
    clean_body: &str,
) -> Option<String> {
    let body = clean_body.trim();
    let lower = body.to_lowercase();

    let rest = strip_command(&lower, body, "portfolio")?;
    let rest_lower = rest.to_lowercase();

    if rest_lower.is_empty() || rest_lower.starts_with("list") {
        return Some(handle_list(services, owner).await);
    }
    if rest_lower.starts_with("summary") {
        return Some(handle_summary(services, owner).await);
    }
    if rest_lower.starts_with("cash") {
        return Some(handle_cash(services, owner, rest[4..].trim()).await);
    }
    if rest_lower.starts_with("buy") {
        return Some(handle_buy(services, owner, rest[3..].trim()).await);
    }
    if rest_lower.starts_with("record") || rest_lower.starts_with("import") {
        return Some(handle_record(services, owner, rest[6..].trim()).await);
    }
    if rest_lower.starts_with("sell") {
        return Some(handle_sell(services, owner, rest[4..].trim()).await);
    }
    if rest_lower.starts_with("price") {
        return Some(handle_price(services, owner, rest[5..].trim()).await);
    }
    if rest_lower.starts_with("refresh") || rest_lower.starts_with("update") {
        return Some(handle_refresh(services, owner).await);
    }
    if rest_lower.starts_with("note") {
        return Some(handle_note(services, owner, rest[4..].trim()).await);
    }
    if rest_lower.starts_with("remove")
        || rest_lower.starts_with("delete")
        || rest_lower.starts_with("rm")
    {
        let symbol = rest.split_whitespace().nth(1).unwrap_or("");
        return Some(handle_remove(services, owner, symbol).await);
    }
    if rest_lower.starts_with("alert") {
        return Some(handle_alert_command(services, owner, rest[5..].trim()).await);
    }

    Some(usage())
}

/// Matches `!portfolio` or a bare `portfolio`, returning the remainder.
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

async fn handle_price(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let mut parts = args.split_whitespace();
    let Some(symbol_raw) = parts.next() else {
        return "⚠️ Usage: <code>!portfolio price &lt;symbol&gt; &lt;price&gt;</code>".to_string();
    };
    let symbol = match normalise_symbol(symbol_raw) {
        Ok(s) => s,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };

    let existing = match services
        .portfolio
        .get_holding_by_symbol(owner, &symbol)
        .await
    {
        Ok(Some(h)) => h,
        _ => return format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol)),
    };

    let Some(price_raw) = parts.next() else {
        return "⚠️ Usage: <code>!portfolio price &lt;symbol&gt; &lt;price&gt;</code>".to_string();
    };
    let exponent = existing.currency_exponent as u32;
    let price_minor = match parse_amount(price_raw, exponent) {
        Ok(p) => p,
        Err(err) => return format!("⚠️ {}.", html_escape(&err.to_string())),
    };

    match services
        .portfolio
        .set_price(owner, &symbol, price_minor)
        .await
    {
        Ok(Some(_holding)) => {
            format!(
                "✅ <b>{}</b> price set to {} {}.",
                html_escape(&symbol),
                html_escape(&format_amount(price_minor, exponent)),
                html_escape(&existing.currency)
            )
        }
        Ok(None) => format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol)),
        Err(err) => {
            error!("Failed to set portfolio price: {:?}", err);
            "❌ Database error saving the price.".to_string()
        }
    }
}

async fn handle_refresh(services: &AppServices, owner: &AccountId) -> String {
    let summary = services.portfolio.refresh_all_for_owner(owner).await;
    if summary.updated.is_empty() && summary.failed.is_empty() {
        return "ℹ️ You have no holdings to refresh.".to_string();
    }
    let mut msg = String::new();
    if !summary.updated.is_empty() {
        msg.push_str(&format!(
            "🔄 <b>Refreshed market prices for {} holding(s):</b> {}<br/>",
            summary.updated.len(),
            summary
                .updated
                .iter()
                .map(|s| format!("<code>{}</code>", html_escape(s)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !summary.failed.is_empty() {
        msg.push_str(&format!(
            "⚠️ <b>Failed to fetch:</b> {}",
            summary
                .failed
                .iter()
                .map(|s| format!("<code>{}</code>", html_escape(s)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    msg
}

async fn handle_note(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let mut parts = args.splitn(2, char::is_whitespace);
    let Some(symbol_raw) = parts.next().filter(|s| !s.is_empty()) else {
        return "⚠️ Usage: <code>!portfolio note &lt;symbol&gt; [text]</code>".to_string();
    };
    let symbol = match normalise_symbol(symbol_raw) {
        Ok(s) => s,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };
    let text = parts.next().map(str::trim).filter(|t| !t.is_empty());

    if let Some(t) = text {
        if t.len() > 500 {
            return "⚠️ Note exceeds maximum length of 500 characters.".to_string();
        }
    }

    match services.portfolio.set_note(owner, &symbol, text).await {
        Ok(Some(_holding)) => match text {
            Some(t) => format!(
                "✅ Note for <b>{}</b> set to <i>{}</i>.",
                html_escape(&symbol),
                html_escape(t)
            ),
            None => format!("✅ Note for <b>{}</b> cleared.", html_escape(&symbol)),
        },
        Ok(None) => format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol)),
        Err(err) => {
            error!("Failed to set portfolio note: {:?}", err);
            "❌ Database error saving the note.".to_string()
        }
    }
}

async fn handle_remove(services: &AppServices, owner: &AccountId, symbol_raw: &str) -> String {
    if symbol_raw.is_empty() {
        return "⚠️ Usage: <code>!portfolio remove &lt;symbol&gt;</code>".to_string();
    }
    let symbol = match normalise_symbol(symbol_raw) {
        Ok(s) => s,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };

    match services.portfolio.delete_holding(owner, &symbol).await {
        Ok(true) => format!("🗑️ Removed <b>{}</b>.", html_escape(&symbol)),
        Ok(false) => format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol)),
        Err(err) => {
            error!("Failed to remove holding: {:?}", err);
            "❌ Database error removing the holding.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_refresh_empty_holdings() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:x"]).await;
        let services = crate::application::AppServices::new(&pool);

        let reply =
            handle_portfolio_command(&services, &AccountId::new("@alice:x"), "!portfolio refresh")
                .await
                .unwrap();
        assert!(reply.contains("no holdings to refresh"));
    }

    #[tokio::test]
    async fn test_record_holding_without_cash() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["@alice:x"]).await;
        let services = crate::application::AppServices::new(&pool);

        let reply = handle_portfolio_command(
            &services,
            &AccountId::new("@alice:x"),
            "!portfolio record AAPL 10 @ 150.00 USD initial",
        )
        .await
        .unwrap();
        assert!(reply.contains("Recorded holding of"));
        assert!(reply.contains("no cash deducted"));

        // Verify holding exists in service
        let h = services
            .portfolio
            .get_holding_by_symbol(&AccountId::new("@alice:x"), "AAPL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(h.symbol, "AAPL");
        assert_eq!(
            h.quantity_minor,
            10 * 10i64.pow(crate::domain::portfolio::models::QUANTITY_EXPONENT)
        );
    }
}
