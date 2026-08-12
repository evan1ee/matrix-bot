//! The cash side of a portfolio: balances, deposits, withdrawals.
//!
//! Separated from `handler.rs` for size, along the seam the module already
//! had. Cash is a hard precondition for buying and never for selling, and
//! `PortfolioRepository::buy` performs its own funding check — nothing here
//! gates a purchase, so this file is only the commands a user runs to move
//! money in and out.

use sqlx::SqlitePool;
use tracing::error;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::{
    exponent_for, format_amount, normalise_currency, parse_amount,
};
use crate::modules::portfolio::handler::broadcast;
use crate::modules::portfolio::models::CashBalance;
use crate::modules::portfolio::repository::{PortfolioRepository, WithdrawOutcome};
use crate::modules::settings::SettingsRepository;
use crate::utils::html_escape;

pub fn cash_event(owner_id: &str, cash: &CashBalance) -> SystemEvent {
    SystemEvent::PortfolioCashUpdated {
        owner_id: owner_id.to_string(),
        currency: cash.currency.clone(),
        balance_minor: cash.balance_minor,
    }
}

/// Parses `<amount> [CUR]` for a deposit/withdraw — the same optional,
/// positional currency default as `parse_buy`, minus the quantity/price/note
/// pieces that don't apply to cash.
fn parse_cash_amount(args: &str, default_currency: &str) -> Result<(i64, String, u32), String> {
    let mut parts = args.split_whitespace();
    let amount_raw = parts.next().ok_or("Amount is required")?;
    let currency_raw = parts.next().map(str::to_uppercase).unwrap_or_else(|| default_currency.to_string());

    let currency = normalise_currency(&currency_raw).map_err(|e| e.to_string())?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(amount_raw, exponent).map_err(|e| e.to_string())?;

    Ok((amount_minor, currency, exponent))
}

pub async fn handle_cash(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let mut parts = args.splitn(2, char::is_whitespace);
    let sub = parts.next().unwrap_or("").to_lowercase();
    let rest = parts.next().unwrap_or("").trim();

    match sub.as_str() {
        "" => show_cash(pool, owner_id).await,
        "deposit" => handle_cash_deposit(pool, owner_id, rest, broadcaster).await,
        "withdraw" => handle_cash_withdraw(pool, owner_id, rest, broadcaster).await,
        _ => "⚠️ Usage: <code>!portfolio cash</code>, \
              <code>!portfolio cash deposit &lt;amount&gt; [CUR]</code>, or \
              <code>!portfolio cash withdraw &lt;amount&gt; [CUR]</code>."
            .to_string(),
    }
}

async fn show_cash(pool: &SqlitePool, owner_id: &str) -> String {
    match PortfolioRepository::list_cash(pool, owner_id).await {
        Ok(balances) if balances.is_empty() => {
            "💵 No cash on record. Add some with <code>!portfolio cash deposit 1000 USD</code>.".to_string()
        }
        Ok(balances) => {
            let mut html = "💵 <b>Your cash:</b><br/>".to_string();
            for b in balances {
                html.push_str(&format!(
                    "• {} {}<br/>",
                    html_escape(&format_amount(b.balance_minor, b.currency_exponent as u32)),
                    html_escape(&b.currency)
                ));
            }
            html
        }
        Err(err) => {
            error!("Failed to list cash balances: {:?}", err);
            "❌ Database error loading your cash.".to_string()
        }
    }
}

async fn handle_cash_deposit(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let default_currency = SettingsRepository::get_or_default(pool, owner_id).await.currency;
    let (amount_minor, currency, exponent) = match parse_cash_amount(args, &default_currency) {
        Ok(v) => v,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Usage: <code>!portfolio cash deposit 1000 USD</code>",
                html_escape(&err)
            )
        }
    };

    match PortfolioRepository::deposit_cash(pool, owner_id, &currency, exponent as i64, amount_minor).await
    {
        Ok(cash) => {
            broadcast(broadcaster, cash_event(owner_id, &cash));
            format!(
                "✅ Deposited <b>{} {}</b>. Cash balance: {} {}.",
                html_escape(&format_amount(amount_minor, exponent)),
                html_escape(&currency),
                html_escape(&format_amount(cash.balance_minor, exponent)),
                html_escape(&currency)
            )
        }
        Err(err) => {
            error!("Failed to deposit cash: {:?}", err);
            "❌ Database error recording the deposit.".to_string()
        }
    }
}

async fn handle_cash_withdraw(
    pool: &SqlitePool,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let default_currency = SettingsRepository::get_or_default(pool, owner_id).await.currency;
    let (amount_minor, currency, exponent) = match parse_cash_amount(args, &default_currency) {
        Ok(v) => v,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Usage: <code>!portfolio cash withdraw 200 USD</code>",
                html_escape(&err)
            )
        }
    };

    match PortfolioRepository::withdraw_cash(pool, owner_id, &currency, amount_minor).await {
        Ok(WithdrawOutcome::Ok(cash)) => {
            broadcast(broadcaster, cash_event(owner_id, &cash));
            format!(
                "✅ Withdrew <b>{} {}</b>. Cash balance: {} {}.",
                html_escape(&format_amount(amount_minor, exponent)),
                html_escape(&currency),
                html_escape(&format_amount(cash.balance_minor, exponent)),
                html_escape(&currency)
            )
        }
        Ok(WithdrawOutcome::InsufficientFunds { available_minor }) => format!(
            "⚠️ Only <b>{} {}</b> available.",
            html_escape(&format_amount(available_minor, exponent)),
            html_escape(&currency)
        ),
        Err(err) => {
            error!("Failed to withdraw cash: {:?}", err);
            "❌ Database error recording the withdrawal.".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cash_amount_grammar() {
        let (amount, currency, exponent) = parse_cash_amount("1000 USD", "CNY").unwrap();
        assert_eq!(amount, 100000);
        assert_eq!(currency, "USD");
        assert_eq!(exponent, 2);
    }

    #[test]
    fn test_parse_cash_amount_currency_is_optional() {
        let (_, currency, _) = parse_cash_amount("500", "CNY").unwrap();
        assert_eq!(currency, "CNY");
    }

    #[test]
    fn test_parse_cash_amount_rejects_bad_input() {
        assert!(parse_cash_amount("", "USD").is_err());
        assert!(parse_cash_amount("-100", "USD").is_err());
        assert!(parse_cash_amount("100 EURO", "USD").is_err());
    }
}
