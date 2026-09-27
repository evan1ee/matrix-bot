//! Trade command handling for portfolio (buy, record/import, sell).

use crate::db::DbPool;
use tracing::error;

use crate::events::EventBroadcaster;
use crate::modules::finance::money::{
    exponent_for, format_amount, looks_like_currency, normalise_currency, parse_amount,
};
use crate::modules::portfolio::models::{normalise_symbol, QUANTITY_EXPONENT};
use crate::modules::portfolio::repository::{BuyOutcome, PortfolioRepository, SellOutcome};
use crate::modules::portfolio::service::{self, PortfolioError, TradeRequest};
use crate::modules::portfolio::view::{format_quantity, signed_amount};
use crate::utils::html_escape;

/// Parses `<symbol> <qty> @ <price> [CUR] [note...]`. Currency is optional and
/// positional, detected exactly like `finance::handler::parse_entry` does —
/// a bare three-letter word right after the price is read as a currency code,
/// anything else starts the note.
pub struct ParsedBuy {
    pub symbol: String,
    pub quantity_minor: i64,
    pub price_minor: i64,
    pub currency: String,
    pub note: Option<String>,
}

pub fn parse_buy(args: &str, default_currency: &str) -> Result<ParsedBuy, String> {
    let mut parts = args.split_whitespace();
    let symbol_raw = parts.next().ok_or("Symbol is required")?;
    let qty_raw = parts.next().ok_or("Quantity is required")?;
    let at = parts.next().ok_or("Expected '@' before the price")?;
    if at != "@" {
        return Err(
            "Expected '@' before the price, e.g. !portfolio buy AAPL 10 @ 150.00".to_string(),
        );
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

    let note = if note_parts.is_empty() {
        None
    } else {
        Some(note_parts.join(" "))
    };

    Ok(ParsedBuy {
        symbol,
        quantity_minor,
        price_minor,
        currency,
        note,
    })
}

pub async fn handle_buy(
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let default_currency = settings.get_or_default(&acct).await.currency;

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
    // The grammar already parsed these into minor units (and the unit tests
    // for `parse_buy` assert on exactly that), so they are re-rendered as
    // decimal strings for the service, which owns the parse-write-announce
    // sequence and takes raw input like every other transport.
    let quantity_str = format_amount(parsed.quantity_minor, QUANTITY_EXPONENT);
    let price_str = format_amount(parsed.price_minor, exponent);

    let outcome = service::buy(
        pool,
        broadcaster,
        owner_id,
        TradeRequest {
            symbol: &parsed.symbol,
            quantity: &quantity_str,
            price: &price_str,
            currency: &parsed.currency,
            note: parsed.note.as_deref(),
            executed_at: None,
        },
    )
    .await;

    match outcome {
        Ok(BuyOutcome::Ok { holding, cash }) => {
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
        Err(PortfolioError::NoteTooLong { max }) => {
            format!("⚠️ Note exceeds {} character limit.", max)
        }
        Err(err) => {
            error!("Failed to record a portfolio buy: {:?}", err);
            "❌ Database error recording the purchase.".to_string()
        }
    }
}

pub async fn handle_record(
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let default_currency = settings.get_or_default(&acct).await.currency;
    let parsed = match parse_buy(args, &default_currency) {
        Ok(p) => p,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };
    let exponent = exponent_for(&parsed.currency);
    let quantity_str = format_amount(parsed.quantity_minor, QUANTITY_EXPONENT);
    let price_str = format_amount(parsed.price_minor, exponent);

    match service::record(
        pool,
        broadcaster,
        owner_id,
        TradeRequest {
            symbol: &parsed.symbol,
            quantity: &quantity_str,
            price: &price_str,
            currency: &parsed.currency,
            note: parsed.note.as_deref(),
            executed_at: None,
        },
    )
    .await
    {
        Ok(Ok(holding)) => {
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
        Err(PortfolioError::NoteTooLong { max }) => {
            format!("⚠️ Note exceeds {} character limit.", max)
        }
        Err(err) => {
            error!("Failed to record holding: {:?}", err);
            "❌ Database error recording position.".to_string()
        }
    }
}

/// Parses `<symbol> <qty> @ <price>`. Unlike buy, the currency is never given —
/// a sale is always in the currency the position is already held in.
pub struct ParsedSell {
    pub symbol: String,
    pub quantity_minor: i64,
    pub price_minor: i64,
}

pub fn parse_sell(
    args: &str,
    exponent_for_symbol: impl Fn(&str) -> Option<u32>,
) -> Result<ParsedSell, String> {
    let mut parts = args.split_whitespace();
    let symbol_raw = parts.next().ok_or("Symbol is required")?;
    let qty_raw = parts.next().ok_or("Quantity is required")?;
    let at = parts.next().ok_or("Expected '@' before the price")?;
    if at != "@" {
        return Err(
            "Expected '@' before the price, e.g. !portfolio sell AAPL 5 @ 180.00".to_string(),
        );
    }
    let price_raw = parts.next().ok_or("Price is required")?;

    let symbol = normalise_symbol(symbol_raw)?;
    let exponent =
        exponent_for_symbol(&symbol).ok_or_else(|| format!("You do not hold {}", symbol))?;
    let price_minor = parse_amount(price_raw, exponent).map_err(|e| e.to_string())?;
    let quantity_minor = parse_amount(qty_raw, QUANTITY_EXPONENT).map_err(|e| e.to_string())?;

    Ok(ParsedSell {
        symbol,
        quantity_minor,
        price_minor,
    })
}

pub async fn handle_sell(
    pool: &DbPool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let symbol_raw = args.split_whitespace().next().unwrap_or("");
    let symbol_for_lookup = normalise_symbol(symbol_raw).ok();
    let existing = match &symbol_for_lookup {
        Some(sym) => PortfolioRepository::find(pool, owner_id, sym)
            .await
            .ok()
            .flatten(),
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

    // `parse_sell` above already split and validated these two tokens (using
    // `existing`'s own currency exponent); they are handed to the service raw
    // rather than re-derived from `parsed`'s already-converted minor units,
    // since the service — not this grammar — owns turning a decimal string
    // into minor units.
    let tokens: Vec<&str> = args.split_whitespace().collect();
    let quantity_raw = tokens.get(1).copied().unwrap_or_default();
    let price_raw = tokens.get(3).copied().unwrap_or_default();

    match service::sell(
        pool,
        broadcaster,
        owner_id,
        &parsed.symbol,
        quantity_raw,
        price_raw,
        None,
    )
    .await
    {
        Ok(SellOutcome::NotFound) => {
            format!("⚠️ You do not hold <b>{}</b>.", html_escape(&parsed.symbol))
        }
        Ok(SellOutcome::ExceedsHolding { held_minor }) => format!(
            "⚠️ You only hold <b>{}</b> of <b>{}</b>.",
            html_escape(&format_quantity(held_minor)),
            html_escape(&parsed.symbol)
        ),
        Ok(SellOutcome::Closed {
            realized_pl_minor,
            cash,
        }) => {
            let exp = existing.currency_exponent as u32;
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
        Ok(SellOutcome::Reduced {
            holding,
            realized_pl_minor,
            cash,
        }) => {
            let exp = holding.currency_exponent as u32;
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

    /// The defect this migration fixes for the Matrix path: before routing
    /// through `service::buy`, a note of any length reached the database.
    #[tokio::test]
    async fn test_a_matrix_buy_enforces_the_note_limit() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let settings = crate::application::settings::SettingsService::new(
            pool.clone(),
            crate::events::EventBroadcaster::default(),
        );

        let over_limit_note = "x".repeat(service::MAX_NOTE_LEN + 1);
        let args = format!("AAPL 10 @ 150.00 USD {}", over_limit_note);

        let reply = handle_buy(&pool, &settings, "@alice:a", &args, None).await;
        assert!(
            reply.contains(&service::MAX_NOTE_LEN.to_string()),
            "the reply must report the character limit: {reply}"
        );
    }
}
