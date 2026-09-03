use sqlx::SqlitePool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::{exponent_for, format_amount, looks_like_currency, normalise_currency, parse_amount};
use crate::modules::portfolio::cash::{cash_event, handle_cash};
use crate::modules::portfolio::view::{format_quantity, handle_list, handle_summary, signed_amount, usage};
use crate::modules::portfolio::models::{normalise_symbol, QUANTITY_EXPONENT};
use crate::modules::portfolio::repository::{BuyOutcome, PortfolioRepository, SellOutcome};
use crate::modules::settings::SettingsRepository;
use crate::utils::html_escape;

pub async fn handle_portfolio_command(
    pool: &SqlitePool,
    owner_id: &str,
    clean_body: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> Option<String> {
    let body = clean_body.trim();
    let lower = body.to_lowercase();

    let rest = strip_command(&lower, body, "portfolio")?;
    let rest_lower = rest.to_lowercase();

    if rest_lower.is_empty() || rest_lower.starts_with("list") {
        return Some(handle_list(pool, owner_id).await);
    }
    if rest_lower.starts_with("summary") {
        return Some(handle_summary(pool, owner_id).await);
    }
    if rest_lower.starts_with("cash") {
        return Some(handle_cash(pool, owner_id, rest[4..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("buy") {
        return Some(handle_buy(pool, owner_id, rest[3..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("record") || rest_lower.starts_with("import") {
        return Some(handle_record(pool, owner_id, rest[6..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("sell") {
        return Some(handle_sell(pool, owner_id, rest[4..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("price") {
        return Some(handle_price(pool, owner_id, rest[5..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("refresh") || rest_lower.starts_with("update") {
        return Some(handle_refresh(pool, owner_id, broadcaster).await);
    }
    if rest_lower.starts_with("note") {
        return Some(handle_note(pool, owner_id, rest[4..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("remove") || rest_lower.starts_with("delete") {
        let symbol = rest.split_whitespace().nth(1).unwrap_or("");
        return Some(handle_remove(pool, owner_id, symbol, broadcaster).await);
    }
    if rest_lower.starts_with("alert") {
        return Some(crate::modules::portfolio::alerts_handler::handle_alert_command(pool, owner_id, rest[5..].trim(), broadcaster).await);
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

/// Parses `<symbol> <qty> @ <price> [CUR] [note...]`. Currency is optional and
/// positional, detected exactly like `finance::handler::parse_entry` does —
/// a bare three-letter word right after the price is read as a currency code,
/// anything else starts the note.
struct ParsedBuy {
    symbol: String,
    quantity_minor: i64,
    price_minor: i64,
    currency: String,
    note: Option<String>,
}

fn parse_buy(args: &str, default_currency: &str) -> Result<ParsedBuy, String> {
    let mut parts = args.split_whitespace();
    let symbol_raw = parts.next().ok_or("Symbol is required")?;
    let qty_raw = parts.next().ok_or("Quantity is required")?;
    let at = parts.next().ok_or("Expected '@' before the price")?;
    if at != "@" {
        return Err("Expected '@' before the price, e.g. !portfolio buy AAPL 10 @ 150.00".to_string());
    }
    let price_raw = parts.next().ok_or("Price is required")?;

    let remainder: Vec<&str> = parts.collect();
    let (currency_raw, note_parts) = match remainder.first() {
        Some(first) if looks_like_currency(first) => (first.to_uppercase(), &remainder[1..]),
        _ => (default_currency.to_string(), &remainder[..]),
    };

    let symbol = normalise_symbol(symbol_raw)?;
    let currency = normalise_currency(&currency_raw).map_err(|e| e.to_string())?;
    let exponent = exponent_for(&currency);
    let price_minor = parse_amount(price_raw, exponent).map_err(|e| e.to_string())?;
    let quantity_minor = parse_amount(qty_raw, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;

    let note = if note_parts.is_empty() { None } else { Some(note_parts.join(" ")) };

    Ok(ParsedBuy { symbol, quantity_minor, price_minor, currency, note })
}

async fn handle_buy(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let default_currency = SettingsRepository::get_or_default(pool, owner_id).await.currency;

    let parsed = match parse_buy(args, &default_currency) {
        Ok(p) => p,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Usage: <code>!portfolio buy AAPL 10 @ 150.00 USD core position</code>",
                html_escape(&err)
            )
        }
    };

    let exponent = exponent_for(&parsed.currency);
    let outcome = PortfolioRepository::buy(
        pool,
        owner_id,
        &parsed.symbol,
        parsed.quantity_minor,
        parsed.price_minor,
        &parsed.currency,
        exponent as i64,
        parsed.note.as_deref(),
    )
    .await;

    match outcome {
        Ok(BuyOutcome::Ok { holding, cash }) => {
            broadcast(broadcaster, SystemEvent::PortfolioHoldingCreated(holding.clone()));
            broadcast(broadcaster, cash_event(owner_id, &cash));
            format!(
                "📈 Bought <b>{} {}</b> @ {} {}. Position: {} @ avg {} {}. Cash remaining: {} {}.",
                html_escape(&format_quantity(parsed.quantity_minor)),
                html_escape(&parsed.symbol),
                html_escape(&format_amount(parsed.price_minor, exponent)),
                html_escape(&parsed.currency),
                html_escape(&format_quantity(holding.quantity_minor)),
                html_escape(&format_amount(holding.avg_cost_minor, exponent)),
                html_escape(&holding.currency),
                html_escape(&format_amount(cash.balance_minor, exponent)),
                html_escape(&cash.currency),
            )
        }
        Ok(BuyOutcome::CurrencyMismatch { existing_currency }) => format!(
            "⚠️ <b>{}</b> is already held in <b>{}</b>. Buy more in {} or remove the position first.",
            html_escape(&parsed.symbol),
            html_escape(&existing_currency),
            html_escape(&existing_currency)
        ),
        Ok(BuyOutcome::InsufficientCash { available_minor, needed_minor }) => format!(
            "⚠️ Need <b>{} {}</b> but only <b>{} {}</b> in cash. Deposit more with \
             <code>!portfolio cash deposit &lt;amount&gt; {}</code>.",
            html_escape(&format_amount(needed_minor, exponent)),
            html_escape(&parsed.currency),
            html_escape(&format_amount(available_minor, exponent)),
            html_escape(&parsed.currency),
            html_escape(&parsed.currency),
        ),
        Err(err) => {
            error!("Failed to record a portfolio buy: {:?}", err);
            "❌ Database error recording the purchase.".to_string()
        }
    }
}

async fn handle_record(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let default_currency = SettingsRepository::get_or_default(pool, owner_id).await.currency;
    let parsed = match parse_buy(args, &default_currency) {
        Ok(p) => p,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };
    let exponent = exponent_for(&parsed.currency);

    match PortfolioRepository::record_holding(
        pool,
        owner_id,
        &parsed.symbol,
        parsed.quantity_minor,
        parsed.price_minor,
        &parsed.currency,
        exponent as i64,
        parsed.note.as_deref(),
    )
    .await
    {
        Ok(Ok(holding)) => {
            broadcast(broadcaster, SystemEvent::PortfolioHoldingCreated(holding.clone()));
            format!(
                "📝 Recorded holding of <b>{} {}</b> @ {} {}. Total position: {} @ avg {} {} (no cash deducted).",
                html_escape(&format_quantity(parsed.quantity_minor)),
                html_escape(&parsed.symbol),
                html_escape(&format_amount(parsed.price_minor, exponent)),
                html_escape(&parsed.currency),
                html_escape(&format_quantity(holding.quantity_minor)),
                html_escape(&format_amount(holding.avg_cost_minor, exponent)),
                html_escape(&holding.currency),
            )
        }
        Ok(Err(existing_currency)) => format!(
            "⚠️ <b>{}</b> is already held in <b>{}</b>.",
            html_escape(&parsed.symbol),
            html_escape(&existing_currency),
        ),
        Err(err) => {
            error!("Failed to record holding: {:?}", err);
            "❌ Database error recording position.".to_string()
        }
    }
}

/// Parses `<symbol> <qty> @ <price>`. Unlike buy, the currency is never given —
/// a sale is always in the currency the position is already held in.
struct ParsedSell {
    symbol: String,
    quantity_minor: i64,
    price_minor: i64,
}

fn parse_sell(args: &str, exponent_for_symbol: impl Fn(&str) -> Option<u32>) -> Result<ParsedSell, String> {
    let mut parts = args.split_whitespace();
    let symbol_raw = parts.next().ok_or("Symbol is required")?;
    let qty_raw = parts.next().ok_or("Quantity is required")?;
    let at = parts.next().ok_or("Expected '@' before the price")?;
    if at != "@" {
        return Err("Expected '@' before the price, e.g. !portfolio sell AAPL 5 @ 180.00".to_string());
    }
    let price_raw = parts.next().ok_or("Price is required")?;

    let symbol = normalise_symbol(symbol_raw)?;
    let exponent = exponent_for_symbol(&symbol).ok_or_else(|| format!("You do not hold {}", symbol))?;
    let price_minor = parse_amount(price_raw, exponent).map_err(|e| e.to_string())?;
    let quantity_minor = parse_amount(qty_raw, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;

    Ok(ParsedSell { symbol, quantity_minor, price_minor })
}

async fn handle_sell(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    // The price has to be parsed against the *held* currency's exponent, so the
    // holding is looked up once up front and reused both to validate the price
    // and to report the currency in the reply.
    let symbol_raw = args.split_whitespace().next().unwrap_or("");
    let symbol_for_lookup = normalise_symbol(symbol_raw).ok();
    let existing = match &symbol_for_lookup {
        Some(sym) => PortfolioRepository::find(pool, owner_id, sym).await.ok().flatten(),
        None => None,
    };
    let currency_exponent = existing.as_ref().map(|h| h.currency_exponent as u32);

    let parsed = match parse_sell(args, |_| currency_exponent) {
        Ok(p) => p,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Usage: <code>!portfolio sell AAPL 5 @ 180.00</code>",
                html_escape(&err)
            )
        }
    };

    let Some(existing) = existing else {
        return format!("⚠️ You do not hold <b>{}</b>.", html_escape(&parsed.symbol));
    };

    match PortfolioRepository::sell(
        pool,
        owner_id,
        &parsed.symbol,
        parsed.quantity_minor,
        parsed.price_minor,
    )
    .await
    {
        Ok(SellOutcome::NotFound) => format!("⚠️ You do not hold <b>{}</b>.", html_escape(&parsed.symbol)),
        Ok(SellOutcome::ExceedsHolding { held_minor }) => format!(
            "⚠️ You only hold <b>{}</b> of <b>{}</b>.",
            html_escape(&format_quantity(held_minor)),
            html_escape(&parsed.symbol)
        ),
        Ok(SellOutcome::Closed { realized_pl_minor, cash }) => {
            let exp = existing.currency_exponent as u32;
            broadcast(
                broadcaster,
                SystemEvent::PortfolioHoldingDeleted { id: existing.id, owner_id: owner_id.to_string() },
            );
            broadcast(broadcaster, cash_event(owner_id, &cash));
            format!(
                "📉 Sold <b>{} {}</b> — position closed. Realized P&amp;L: {} {}. Cash balance: {} {}.",
                html_escape(&format_quantity(parsed.quantity_minor)),
                html_escape(&parsed.symbol),
                signed_amount(realized_pl_minor, exp),
                html_escape(&existing.currency),
                html_escape(&format_amount(cash.balance_minor, exp)),
                html_escape(&cash.currency),
            )
        }
        Ok(SellOutcome::Reduced { holding, realized_pl_minor, cash }) => {
            let exp = holding.currency_exponent as u32;
            broadcast(broadcaster, SystemEvent::PortfolioHoldingUpdated(holding.clone()));
            broadcast(broadcaster, cash_event(owner_id, &cash));
            format!(
                "📉 Sold <b>{} {}</b>. Remaining: {} @ avg {} {}. Realized P&amp;L: {} {}. Cash balance: {} {}.",
                html_escape(&format_quantity(parsed.quantity_minor)),
                html_escape(&parsed.symbol),
                html_escape(&format_quantity(holding.quantity_minor)),
                html_escape(&format_amount(holding.avg_cost_minor, exp)),
                html_escape(&holding.currency),
                signed_amount(realized_pl_minor, exp),
                html_escape(&holding.currency),
                html_escape(&format_amount(cash.balance_minor, exp)),
                html_escape(&cash.currency),
            )
        }
        Err(err) => {
            error!("Failed to record a portfolio sell: {:?}", err);
            "❌ Database error recording the sale.".to_string()
        }
    }
}

async fn handle_price(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let mut parts = args.split_whitespace();
    let Some(symbol_raw) = parts.next() else {
        return "⚠️ Usage: <code>!portfolio price &lt;symbol&gt; &lt;price&gt;</code>".to_string();
    };
    let symbol = match normalise_symbol(symbol_raw) {
        Ok(s) => s,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };

    let Some(existing) = PortfolioRepository::find(pool, owner_id, &symbol).await.ok().flatten() else {
        return format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol));
    };

    let Some(price_raw) = parts.next() else {
        return "⚠️ Usage: <code>!portfolio price &lt;symbol&gt; &lt;price&gt;</code>".to_string();
    };
    let exponent = existing.currency_exponent as u32;
    let price_minor = match parse_amount(price_raw, exponent) {
        Ok(p) => p,
        Err(err) => return format!("⚠️ {}.", html_escape(&err.to_string())),
    };

    match PortfolioRepository::set_price(pool, owner_id, &symbol, price_minor).await {
        Ok(Some(holding)) => {
            broadcast(broadcaster, SystemEvent::PortfolioHoldingUpdated(holding.clone()));
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

async fn handle_refresh(
    pool: &SqlitePool,
    owner_id: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let summary = super::price_feed::refresh_all_for_owner(pool, broadcaster, owner_id).await;
    if summary.updated.is_empty() && summary.failed.is_empty() {
        return "ℹ️ You have no holdings to refresh.".to_string();
    }
    let mut msg = String::new();
    if !summary.updated.is_empty() {
        msg.push_str(&format!(
            "🔄 <b>Refreshed market prices for {} holding(s):</b> {}<br/>",
            summary.updated.len(),
            summary.updated.iter().map(|s| format!("<code>{}</code>", html_escape(s))).collect::<Vec<_>>().join(", ")
        ));
    }
    if !summary.failed.is_empty() {
        msg.push_str(&format!(
            "⚠️ <b>Failed to fetch:</b> {}",
            summary.failed.iter().map(|s| format!("<code>{}</code>", html_escape(s))).collect::<Vec<_>>().join(", ")
        ));
    }
    msg
}

async fn handle_note(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let mut parts = args.splitn(2, char::is_whitespace);
    let Some(symbol_raw) = parts.next().filter(|s| !s.is_empty()) else {
        return "⚠️ Usage: <code>!portfolio note &lt;symbol&gt; [text]</code>".to_string();
    };
    let symbol = match normalise_symbol(symbol_raw) {
        Ok(s) => s,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };
    let text = parts.next().map(str::trim).filter(|t| !t.is_empty());

    match PortfolioRepository::set_note(pool, owner_id, &symbol, text).await {
        Ok(Some(holding)) => {
            broadcast(broadcaster, SystemEvent::PortfolioHoldingUpdated(holding));
            match text {
                Some(t) => format!("✅ Note for <b>{}</b> set to <i>{}</i>.", html_escape(&symbol), html_escape(t)),
                None => format!("✅ Note for <b>{}</b> cleared.", html_escape(&symbol)),
            }
        }
        Ok(None) => format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol)),
        Err(err) => {
            error!("Failed to set portfolio note: {:?}", err);
            "❌ Database error saving the note.".to_string()
        }
    }
}

async fn handle_remove(
    pool: &SqlitePool,
    owner_id: &str,
    symbol_raw: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    if symbol_raw.is_empty() {
        return "⚠️ Usage: <code>!portfolio remove &lt;symbol&gt;</code>".to_string();
    }
    let symbol = match normalise_symbol(symbol_raw) {
        Ok(s) => s,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };

    let Some(existing) = PortfolioRepository::find(pool, owner_id, &symbol).await.ok().flatten() else {
        return format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol));
    };

    match PortfolioRepository::delete(pool, owner_id, &symbol).await {
        Ok(true) => {
            broadcast(
                broadcaster,
                SystemEvent::PortfolioHoldingDeleted { id: existing.id, owner_id: owner_id.to_string() },
            );
            format!("🗑️ Removed <b>{}</b>.", html_escape(&symbol))
        }
        Ok(false) => format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol)),
        Err(err) => {
            error!("Failed to remove holding: {:?}", err);
            "❌ Database error removing the holding.".to_string()
        }
    }
}

pub fn broadcast(broadcaster: Option<&EventBroadcaster>, event: SystemEvent) {
    if let Some(b) = broadcaster {
        b.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_buy_grammar() {
        let parsed = parse_buy("AAPL 10 @ 150.00 USD core position", "USD").unwrap();
        assert_eq!(parsed.symbol, "AAPL");
        assert_eq!(parsed.price_minor, 15000);
        assert_eq!(parsed.currency, "USD");
        assert_eq!(parsed.note.as_deref(), Some("core position"));
    }

    #[test]
    fn test_parse_buy_currency_is_optional() {
        let parsed = parse_buy("AAPL 10 @ 150.00", "CNY").unwrap();
        assert_eq!(parsed.currency, "CNY");
        assert!(parsed.note.is_none());
    }

    /// Without a recognised currency token, everything after the price is note.
    #[test]
    fn test_parse_buy_note_without_currency() {
        let parsed = parse_buy("AAPL 10 @ 150.00 for the long haul", "USD").unwrap();
        assert_eq!(parsed.currency, "USD");
        assert_eq!(parsed.note.as_deref(), Some("for the long haul"));
    }

    #[test]
    fn test_parse_buy_requires_the_at_sign() {
        assert!(parse_buy("AAPL 10 150.00", "USD").is_err());
        assert!(parse_buy("AAPL 10 x 150.00", "USD").is_err());
    }

    #[test]
    fn test_parse_buy_rejects_bad_quantity() {
        assert!(parse_buy("AAPL -10 @ 150.00", "USD").is_err());
        assert!(parse_buy("AAPL abc @ 150.00", "USD").is_err());
    }

    #[test]
    fn test_parse_sell_grammar() {
        let parsed = parse_sell("AAPL 5 @ 180.00", |_| Some(2)).unwrap();
        assert_eq!(parsed.symbol, "AAPL");
        assert_eq!(parsed.quantity_minor, 5 * 10i64.pow(QUANTITY_EXPONENT));
        assert_eq!(parsed.price_minor, 18000);
    }

    #[test]
    fn test_parse_sell_requires_the_at_sign() {
        assert!(parse_sell("AAPL 5 180.00", |_| Some(2)).is_err());
    }

    #[test]
    fn test_parse_sell_unknown_symbol_is_rejected_before_price_parsing() {
        assert!(parse_sell("AAPL 5 @ 180.00", |_| None).is_err());
    }

    #[test]
    fn test_format_quantity_trims_trailing_zeros() {
        assert_eq!(format_quantity(10 * 10i64.pow(QUANTITY_EXPONENT)), "10");
        assert_eq!(format_quantity(150 * 10i64.pow(QUANTITY_EXPONENT) / 100), "1.5");
        assert_eq!(format_quantity(1), "0.00000001");
    }

    #[test]
    fn test_signed_amount_shows_the_sign() {
        assert_eq!(signed_amount(1550, 2), "+15.50");
        assert_eq!(signed_amount(-1550, 2), "-15.50");
        assert_eq!(signed_amount(0, 2), "+0.00");
    }

    #[tokio::test]
    async fn test_refresh_empty_holdings() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();

        let reply = handle_portfolio_command(&pool, "@alice:x", "!portfolio refresh", None)
            .await
            .unwrap();
        assert!(reply.contains("no holdings to refresh"));
    }

    #[tokio::test]
    async fn test_record_holding_without_cash() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();

        let reply = handle_portfolio_command(&pool, "@alice:x", "!portfolio record AAPL 10 @ 150.00 USD initial", None)
            .await
            .unwrap();
        assert!(reply.contains("Recorded holding of"));
        assert!(reply.contains("no cash deducted"));

        // Verify holding exists in DB
        let h = PortfolioRepository::find(&pool, "@alice:x", "AAPL").await.unwrap().unwrap();
        assert_eq!(h.symbol, "AAPL");
        assert_eq!(h.quantity_minor, 10 * 10i64.pow(QUANTITY_EXPONENT));
    }
}
