//! The cash side of a portfolio: balances, deposits, withdrawals.
//!
//! Separated from `handler.rs` for size, along the seam the module already
//! had. Cash is a hard precondition for buying and never for selling, and
//! `trade::buy` performs its own funding check — nothing here gates a
//! purchase, so this file is only the commands a user runs to move money in
//! and out.

use crate::db::DbPool;
use tracing::error;

use super::cash_repo;
use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::models::{Transaction, TransactionSource};
use crate::modules::finance::money::{
    exponent_for, format_amount, normalise_currency, parse_amount,
};
use crate::modules::finance::service as finance_service;
use crate::modules::portfolio::models::CashBalance;
use crate::modules::portfolio::repository::WithdrawOutcome;
use crate::modules::portfolio::service;
use crate::utils::html_escape;

pub fn cash_event(owner_id: &str, cash: &CashBalance) -> SystemEvent {
    SystemEvent::PortfolioCashUpdated {
        owner_id: owner_id.to_string(),
        currency: cash.currency.clone(),
        balance_minor: cash.balance_minor,
    }
}

/// Posts a cash deposit or withdrawal to the finance ledger through the
/// shared finance service, so portfolio funding reflects in liquid cash
/// balances and, like every other write through the service, broadcasts
/// `TransactionCreated` itself rather than leaving each call site to do it —
/// a portfolio deposit or withdrawal used to write straight to
/// `FinanceRepository` and never announce itself to an open browser.
pub async fn record_portfolio_cash_transaction(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    action: &str,
    amount_minor: i64,
    currency: &str,
    exponent: i64,
    cash_id: i64,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<Transaction> {
    let (direction, note) = match action {
        "deposit" => ("expense", "Portfolio cash deposit"),
        "withdraw" => ("income", "Portfolio cash withdrawal"),
        _ => return None,
    };

    finance_service::create_transaction(
        pool,
        broadcaster,
        owner_id,
        finance_service::NewTransaction {
            direction,
            amount_minor,
            currency,
            currency_exponent: exponent,
            // The category is inferred from the action, not typed by a user,
            // so an unknown name is created rather than refused — matching
            // what `find_or_create_category` did here before this call was
            // routed through the shared service.
            category: Some("Investments"),
            note: Some(note),
            room_id: "web",
            sender_name: "Portfolio",
            occurred_at: executed_at.unwrap_or_else(chrono::Utc::now),
            source: Some((TransactionSource::PortfolioCash.as_str(), cash_id)),
            create_missing_category: true,
        },
    )
    .await
    .map_err(|err| error!("Failed to record portfolio cash transaction: {:?}", err))
    .ok()
}

/// Parses `<amount> [CUR]` for a deposit/withdraw — the same optional,
/// positional currency default as `parse_buy`, minus the quantity/price/note
/// pieces that don't apply to cash.
fn parse_cash_amount(args: &str, default_currency: &str) -> Result<(i64, String, u32), String> {
    let mut parts = args.split_whitespace();
    let amount_raw = parts.next().ok_or("Amount is required")?;
    let currency_raw = parts
        .next()
        .map(str::to_uppercase)
        .unwrap_or_else(|| default_currency.to_string());

    let currency = normalise_currency(&currency_raw).map_err(|e| e.to_string())?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(amount_raw, exponent).map_err(|e| e.to_string())?;

    Ok((amount_minor, currency, exponent))
}

pub async fn handle_cash(
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let mut parts = args.splitn(2, char::is_whitespace);
    let sub = parts.next().unwrap_or("").to_lowercase();
    let rest = parts.next().unwrap_or("").trim();

    match sub.as_str() {
        "" => show_cash(pool, owner_id).await,
        "deposit" => handle_cash_deposit(pool, settings, owner_id, rest, broadcaster).await,
        "withdraw" => handle_cash_withdraw(pool, settings, owner_id, rest, broadcaster).await,
        _ => "⚠️ Usage: <code>!portfolio cash</code>, \
              <code>!portfolio cash deposit &lt;amount&gt; [CUR]</code>, or \
              <code>!portfolio cash withdraw &lt;amount&gt; [CUR]</code>."
            .to_string(),
    }
}

async fn show_cash(pool: &DbPool, owner_id: &str) -> String {
    match cash_repo::list_cash(pool, owner_id).await {
        Ok(balances) if balances.is_empty() => {
            "💵 No cash on record. Add some with <code>!portfolio cash deposit 1000 USD</code>."
                .to_string()
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
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let default_currency = settings.get_or_default(&acct).await.currency;
    let (amount_minor, currency, exponent) = match parse_cash_amount(args, &default_currency) {
        Ok(v) => v,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Usage: <code>!portfolio cash deposit 1000 USD</code>",
                html_escape(&err)
            )
        }
    };
    // `parse_cash_amount`'s own tests destructure its tuple return, so it
    // cannot also hand back the raw amount string — re-render it instead of
    // widening that signature.
    let amount_str = format_amount(amount_minor, exponent);

    match service::deposit_cash(pool, broadcaster, owner_id, &currency, &amount_str, None).await {
        Ok(cash) => {
            // The service broadcasts `PortfolioCashUpdated` itself; this call
            // site only needs to post the finance-ledger transaction below.
            record_portfolio_cash_transaction(
                pool,
                broadcaster,
                owner_id,
                "deposit",
                amount_minor,
                &currency,
                exponent as i64,
                cash.id,
                None,
            )
            .await;
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
    pool: &DbPool,
    settings: &crate::application::settings::SettingsService,
    owner_id: &str,
    args: &str,
    broadcaster: Option<&EventBroadcaster>,
) -> String {
    let acct = crate::domain::accounts::AccountId::from(owner_id);
    let default_currency = settings.get_or_default(&acct).await.currency;
    let (amount_minor, currency, exponent) = match parse_cash_amount(args, &default_currency) {
        Ok(v) => v,
        Err(err) => {
            return format!(
                "⚠️ {}.<br/>Usage: <code>!portfolio cash withdraw 200 USD</code>",
                html_escape(&err)
            )
        }
    };
    let amount_str = format_amount(amount_minor, exponent);

    match service::withdraw_cash(pool, broadcaster, owner_id, &currency, &amount_str, None).await {
        Ok(WithdrawOutcome::Ok(cash)) => {
            // The service broadcasts `PortfolioCashUpdated` itself; this call
            // site only needs to post the finance-ledger transaction below.
            record_portfolio_cash_transaction(
                pool,
                broadcaster,
                owner_id,
                "withdraw",
                amount_minor,
                &currency,
                exponent as i64,
                cash.id,
                None,
            )
            .await;
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

    /// The bug this fix exists to close: a portfolio deposit wrote straight to
    /// `FinanceRepository` and never announced itself, so the resulting
    /// finance transaction was invisible to an open browser until reload.
    /// Routing it through `finance::service::create_transaction` must
    /// broadcast `TransactionCreated` like every other write through the
    /// service.
    #[tokio::test]
    async fn test_portfolio_cash_deposit_broadcasts_a_transaction_created() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();
        let settings =
            crate::application::settings::SettingsService::new(pool.clone(), broadcaster.clone());

        let reply = handle_cash(
            &pool,
            &settings,
            "@alice:a",
            "deposit 1000 USD",
            Some(&broadcaster),
        )
        .await;
        assert!(reply.contains("Deposited"), "unexpected reply: {reply}");

        let mut saw_transaction_created = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, SystemEvent::TransactionCreated(_)) {
                saw_transaction_created = true;
            }
        }
        assert!(
            saw_transaction_created,
            "a portfolio cash deposit must broadcast TransactionCreated"
        );
    }
}
