use crate::application::settings::SettingsService;
use crate::db::DbPool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::{format_amount, parse_amount};
use crate::modules::portfolio::cash::handle_cash;
use crate::modules::portfolio::models::normalise_symbol;
use crate::modules::portfolio::repository::PortfolioRepository;
use crate::modules::portfolio::trade_cmd::{handle_buy, handle_record, handle_sell};
use crate::modules::portfolio::view::{handle_list, handle_summary, usage};
use crate::utils::html_escape;

pub async fn handle_portfolio_command(
    pool: &DbPool,
    settings: &SettingsService,
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
        return Some(handle_cash(pool, settings, owner_id, rest[4..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("buy") {
        return Some(handle_buy(pool, settings, owner_id, rest[3..].trim(), broadcaster).await);
    }
    if rest_lower.starts_with("record") || rest_lower.starts_with("import") {
        return Some(handle_record(pool, settings, owner_id, rest[6..].trim(), broadcaster).await);
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
    if rest_lower.starts_with("remove")
        || rest_lower.starts_with("delete")
        || rest_lower.starts_with("rm")
    {
        let symbol = rest.split_whitespace().nth(1).unwrap_or("");
        return Some(handle_remove(pool, owner_id, symbol, broadcaster).await);
    }
    if rest_lower.starts_with("alert") {
        return Some(
            crate::modules::portfolio::alerts_handler::handle_alert_command(
                pool,
                settings,
                owner_id,
                rest[5..].trim(),
                broadcaster,
            )
            .await,
        );
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

async fn handle_price(
    pool: &DbPool,
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

    let Some(existing) = PortfolioRepository::find(pool, owner_id, &symbol)
        .await
        .ok()
        .flatten()
    else {
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
            broadcast(
                broadcaster,
                SystemEvent::PortfolioHoldingUpdated(holding.clone()),
            );
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
    pool: &DbPool,
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

async fn handle_note(
    pool: &DbPool,
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

    if let Some(t) = text {
        if t.len() > 500 {
            return "⚠️ Note exceeds maximum length of 500 characters.".to_string();
        }
    }

    match PortfolioRepository::set_note(pool, owner_id, &symbol, text).await {
        Ok(Some(holding)) => {
            broadcast(broadcaster, SystemEvent::PortfolioHoldingUpdated(holding));
            match text {
                Some(t) => format!(
                    "✅ Note for <b>{}</b> set to <i>{}</i>.",
                    html_escape(&symbol),
                    html_escape(t)
                ),
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
    pool: &DbPool,
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

    let Some(existing) = PortfolioRepository::find(pool, owner_id, &symbol)
        .await
        .ok()
        .flatten()
    else {
        return format!("⚠️ You do not hold <b>{}</b>.", html_escape(&symbol));
    };

    match PortfolioRepository::delete(pool, owner_id, &symbol).await {
        Ok(true) => {
            broadcast(
                broadcaster,
                SystemEvent::PortfolioHoldingDeleted {
                    id: existing.id,
                    owner_id: owner_id.to_string(),
                },
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
    use crate::modules::portfolio::models::QUANTITY_EXPONENT;
    use crate::modules::portfolio::view::{format_quantity, signed_amount};

    #[test]
    fn test_format_quantity_trims_trailing_zeros() {
        assert_eq!(format_quantity(10 * 10i64.pow(QUANTITY_EXPONENT)), "10");
        assert_eq!(
            format_quantity(150 * 10i64.pow(QUANTITY_EXPONENT) / 100),
            "1.5"
        );
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let reply =
            handle_portfolio_command(&pool, &settings, "@alice:x", "!portfolio refresh", None)
                .await
                .unwrap();
        assert!(reply.contains("no holdings to refresh"));
    }

    #[tokio::test]
    async fn test_record_holding_without_cash() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let reply = handle_portfolio_command(
            &pool,
            &settings,
            "@alice:x",
            "!portfolio record AAPL 10 @ 150.00 USD initial",
            None,
        )
        .await
        .unwrap();
        assert!(reply.contains("Recorded holding of"));
        assert!(reply.contains("no cash deducted"));

        // Verify holding exists in DB
        let h = PortfolioRepository::find(&pool, "@alice:x", "AAPL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(h.symbol, "AAPL");
        assert_eq!(h.quantity_minor, 10 * 10i64.pow(QUANTITY_EXPONENT));
    }
}
