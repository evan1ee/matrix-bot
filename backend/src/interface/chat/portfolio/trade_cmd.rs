//! Trade command handling for portfolio (buy, record/import, sell).

use tracing::error;

use crate::application::AppServices;
use crate::domain::accounts::AccountId;
use crate::domain::finance::money::{
    exponent_for, format_amount, looks_like_currency, normalise_currency, parse_amount,
};
use crate::domain::portfolio::models::{
    normalise_symbol, BuyOutcome, SellOutcome, QUANTITY_EXPONENT,
};
use crate::domain::portfolio::TradeRequest;
use crate::interface::chat::portfolio::view::{format_quantity, signed_amount};
use crate::utils::html::html_escape;

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

pub async fn handle_buy(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let default_currency = services.settings.get_or_default(owner).await.currency;

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
    let quantity_str = format_amount(parsed.quantity_minor, QUANTITY_EXPONENT);
    let price_str = format_amount(parsed.price_minor, exponent);

    let outcome = services
        .portfolio
        .buy(
            owner,
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
        Err(crate::application::AppError::Validation(msg)) => {
            format!("⚠️ {}", html_escape(&msg))
        }
        Err(err) => {
            error!("Failed to record a portfolio buy: {:?}", err);
            "❌ Database error recording the purchase.".to_string()
        }
    }
}

pub async fn handle_record(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let default_currency = services.settings.get_or_default(owner).await.currency;
    let parsed = match parse_buy(args, &default_currency) {
        Ok(p) => p,
        Err(err) => return format!("⚠️ {}.", html_escape(&err)),
    };
    let exponent = exponent_for(&parsed.currency);
    let quantity_str = format_amount(parsed.quantity_minor, QUANTITY_EXPONENT);
    let price_str = format_amount(parsed.price_minor, exponent);

    match services
        .portfolio
        .record(
            owner,
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
        Err(crate::application::AppError::Validation(msg)) => {
            format!("⚠️ {}", html_escape(&msg))
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

pub async fn handle_sell(services: &AppServices, owner: &AccountId, args: &str) -> String {
    let symbol_raw = args.split_whitespace().next().unwrap_or("");
    let symbol_for_lookup = normalise_symbol(symbol_raw).ok();
    let existing = match &symbol_for_lookup {
        Some(sym) => services
            .portfolio
            .get_holding_by_symbol(owner, sym)
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

    let tokens: Vec<&str> = args.split_whitespace().collect();
    let quantity_raw = tokens.get(1).copied().unwrap_or_default();
    let price_raw = tokens.get(3).copied().unwrap_or_default();

    match services
        .portfolio
        .sell(owner, &parsed.symbol, quantity_raw, price_raw, None)
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
        let p = parse_buy("AAPL 10 @ 150.00 USD core position", "EUR").unwrap();
        assert_eq!(p.symbol, "AAPL");
        assert_eq!(p.quantity_minor, 10 * 10i64.pow(8));
        assert_eq!(p.price_minor, 150_00);
        assert_eq!(p.currency, "USD");
        assert_eq!(p.note.as_deref(), Some("core position"));

        let p2 = parse_buy("BTC 0.5 @ 60000", "USD").unwrap();
        assert_eq!(p2.symbol, "BTC");
        assert_eq!(p2.quantity_minor, 50_000_000);
        assert_eq!(p2.price_minor, 60000_00);
        assert_eq!(p2.currency, "USD");
        assert_eq!(p2.note, None);
    }

    #[test]
    fn test_parse_sell_grammar() {
        let p = parse_sell("AAPL 5 @ 180.00", |_| Some(2)).unwrap();
        assert_eq!(p.symbol, "AAPL");
        assert_eq!(p.quantity_minor, 5 * 10i64.pow(8));
        assert_eq!(p.price_minor, 180_00);
    }
}
