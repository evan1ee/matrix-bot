//! What it means to buy, sell, record, or move cash in a portfolio,
//! independent of how the request arrived.
//!
//! Three transports perform these operations — the REST API, a Matrix
//! command, and an AI tool call — and each repeated the same sequence by
//! hand: `normalise_symbol`, `normalise_currency`, `exponent_for`,
//! `parse_amount` for the price, `parse_amount` for the quantity at
//! `QUANTITY_EXPONENT`, then a write, then emitting events. Their copies had
//! drifted: the note-length limit existed only on the REST path, and the same
//! buy announced itself as a creation from REST and Matrix but an update from
//! the AI tool — neither is reliably right, since a buy of a symbol not
//! already held is a creation and a top-up is an update, regardless of who
//! asked.
//!
//! So the decisions live here and the transports parse their own syntax and
//! render their own replies. This module returns data and typed errors; it
//! renders nothing.
//!
//! `trade::{buy_at, sell_at, record_holding, record_holding_action}` and
//! `cash_repo::{deposit_cash_at, withdraw_cash_at}` still own the transaction
//! and the audit-log write — this module sits above them, not in place of
//! them.

use crate::db::DbPool;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::portfolio::cash::cash_event;
use crate::modules::portfolio::cash_repo;
use crate::modules::portfolio::models::{
    normalise_symbol, BuyOutcome, CashBalance, Holding, SellOutcome, WithdrawOutcome,
    QUANTITY_EXPONENT,
};
use crate::modules::portfolio::repository::PortfolioRepository;
use crate::modules::portfolio::trade;

/// Maximum allowed length for a holding's note.
pub const MAX_NOTE_LEN: usize = 500;

/// A buy or record to perform. The caller has already split its own syntax
/// (a `!portfolio buy` command's `@` separator, a JSON body's fields) out of
/// whatever it arrived in, but the numeric and symbol fields stay raw strings
/// — parsing them is part of what was duplicated.
pub struct TradeRequest<'a> {
    pub symbol: &'a str,
    pub quantity: &'a str,
    pub price: &'a str,
    pub currency: &'a str,
    pub note: Option<&'a str>,
    /// Backdates the trade. `None` means now.
    pub executed_at: Option<DateTime<Utc>>,
}

/// Why a write was refused.
///
/// Variants rather than a rendered string: the Matrix path answers in HTML
/// with `<code>` markup, the REST path answers in JSON, and the AI tool
/// answers in a plain `Err(String)`, so whoever renders has to be the one
/// that knows which it is.
#[derive(Debug)]
pub enum PortfolioError {
    NoteTooLong {
        max: usize,
    },
    /// A symbol, currency or amount that could not be parsed. Carries the
    /// message the parser produced, which already says which field and why.
    Invalid(String),
    Db(anyhow::Error),
}

impl std::fmt::Display for PortfolioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortfolioError::NoteTooLong { max } => {
                write!(f, "Note exceeds {} character limit", max)
            }
            PortfolioError::Invalid(msg) => write!(f, "{}", msg),
            PortfolioError::Db(_) => write!(f, "Internal error"),
        }
    }
}

/// Checks a note's length and normalises it.
///
/// A whitespace-only note becomes `None` rather than an empty string, so a
/// blank field submitted by a form stores nothing rather than storing something
/// that renders as nothing. The REST path did this and the other two did not,
/// which is the sort of difference this module exists to remove.
fn validate_note(note: Option<&str>) -> Result<Option<&str>, PortfolioError> {
    match note {
        Some(n) if n.len() > MAX_NOTE_LEN => Err(PortfolioError::NoteTooLong { max: MAX_NOTE_LEN }),
        Some(n) if n.trim().is_empty() => Ok(None),
        other => Ok(other),
    }
}

/// The pieces every buy/record shares once parsed: a normalised symbol and
/// currency, the currency's decimal exponent, and both amounts converted to
/// minor units.
struct ParsedTrade {
    symbol: String,
    currency: String,
    exponent: u32,
    price_minor: i64,
    quantity_minor: i64,
}

fn parse_trade(req: &TradeRequest<'_>) -> Result<ParsedTrade, PortfolioError> {
    let symbol = normalise_symbol(req.symbol).map_err(PortfolioError::Invalid)?;
    let currency =
        normalise_currency(req.currency).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let price_minor =
        parse_amount(req.price, exponent).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let quantity_minor = parse_amount(req.quantity, QUANTITY_EXPONENT)
        .map_err(|e| PortfolioError::Invalid(e.to_string()))?;

    Ok(ParsedTrade {
        symbol,
        currency,
        exponent,
        price_minor,
        quantity_minor,
    })
}

/// Buys — or adds to — a position: validates, writes through
/// [`trade::buy_at`], then broadcasts.
///
/// The event describes what happened, not which transport asked: whether the
/// owner already held this symbol is checked *before* the write, since
/// `buy_at` itself folds "opened" and "topped up" into one `BuyOutcome::Ok`
/// and does not say which one it was.
pub async fn buy(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    req: TradeRequest<'_>,
) -> Result<BuyOutcome, PortfolioError> {
    let note = validate_note(req.note)?;
    let parsed = parse_trade(&req)?;

    let already_held = PortfolioRepository::find(pool, owner_id, &parsed.symbol)
        .await
        .map_err(PortfolioError::Db)?
        .is_some();

    let outcome = trade::buy_at(
        pool,
        owner_id,
        &parsed.symbol,
        parsed.quantity_minor,
        parsed.price_minor,
        &parsed.currency,
        parsed.exponent as i64,
        note,
        req.executed_at,
    )
    .await
    .map_err(PortfolioError::Db)?;

    if let BuyOutcome::Ok {
        ref holding,
        ref cash,
    } = outcome
    {
        if let Some(b) = broadcaster {
            b.send(if already_held {
                SystemEvent::PortfolioHoldingUpdated(holding.clone())
            } else {
                SystemEvent::PortfolioHoldingCreated(holding.clone())
            });
            b.send(cash_event(owner_id, cash));
        }
    }

    Ok(outcome)
}

/// Sells against an existing holding, then broadcasts.
///
/// Unlike a buy, a sale carries no currency of its own — it is always in
/// whatever the position is already held in — so the exponent used to parse
/// `price` comes from the holding, found by `symbol` before either amount is
/// parsed. A symbol never held cannot be priced at all, so that lookup
/// returning nothing is reported as `SellOutcome::NotFound` rather than a
/// `PortfolioError`, the same "return the outcome enum" contract `buy` and
/// `record` follow.
pub async fn sell(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    symbol: &str,
    quantity: &str,
    price: &str,
    executed_at: Option<DateTime<Utc>>,
) -> Result<SellOutcome, PortfolioError> {
    let symbol = normalise_symbol(symbol).map_err(PortfolioError::Invalid)?;

    let existing = PortfolioRepository::find(pool, owner_id, &symbol)
        .await
        .map_err(PortfolioError::Db)?;
    let Some(existing) = existing else {
        return Ok(SellOutcome::NotFound);
    };
    let exponent = existing.currency_exponent as u32;

    let price_minor =
        parse_amount(price, exponent).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let quantity_minor = parse_amount(quantity, QUANTITY_EXPONENT)
        .map_err(|e| PortfolioError::Invalid(e.to_string()))?;

    let outcome = trade::sell_at(
        pool,
        owner_id,
        &symbol,
        quantity_minor,
        price_minor,
        executed_at,
    )
    .await
    .map_err(PortfolioError::Db)?;

    if let Some(b) = broadcaster {
        match &outcome {
            SellOutcome::Closed { cash, .. } => {
                b.send(SystemEvent::PortfolioHoldingDeleted {
                    id: existing.id,
                    owner_id: owner_id.to_string(),
                });
                b.send(cash_event(owner_id, cash));
            }
            SellOutcome::Reduced { holding, cash, .. } => {
                b.send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
                b.send(cash_event(owner_id, cash));
            }
            SellOutcome::NotFound | SellOutcome::ExceedsHolding { .. } => {}
        }
    }

    Ok(outcome)
}

/// Records an already-owned position without touching cash, then broadcasts.
///
/// Same insert-or-update shape as `buy` (`trade::record_holding` folds
/// "opened" and "topped up" together exactly like `buy_at` does), so it gets
/// the same creation-vs-update event logic and for the same reason.
pub async fn record(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    req: TradeRequest<'_>,
) -> Result<Result<Holding, String>, PortfolioError> {
    let note = validate_note(req.note)?;
    let parsed = parse_trade(&req)?;

    let already_held = PortfolioRepository::find(pool, owner_id, &parsed.symbol)
        .await
        .map_err(PortfolioError::Db)?
        .is_some();

    let result = trade::record_holding(
        pool,
        owner_id,
        &parsed.symbol,
        parsed.quantity_minor,
        parsed.price_minor,
        &parsed.currency,
        parsed.exponent as i64,
        note,
    )
    .await
    .map_err(PortfolioError::Db)?;

    if let Ok(ref holding) = result {
        if let Some(b) = broadcaster {
            b.send(if already_held {
                SystemEvent::PortfolioHoldingUpdated(holding.clone())
            } else {
                SystemEvent::PortfolioHoldingCreated(holding.clone())
            });
        }
    }

    Ok(result)
}

/// Deposits cash, then broadcasts.
pub async fn deposit_cash(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    currency: &str,
    amount: &str,
    executed_at: Option<DateTime<Utc>>,
) -> Result<CashBalance, PortfolioError> {
    let currency =
        normalise_currency(currency).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let amount_minor =
        parse_amount(amount, exponent).map_err(|e| PortfolioError::Invalid(e.to_string()))?;

    let cash = cash_repo::deposit_cash_at(
        pool,
        owner_id,
        &currency,
        exponent as i64,
        amount_minor,
        executed_at,
    )
    .await
    .map_err(PortfolioError::Db)?;

    if let Some(b) = broadcaster {
        b.send(cash_event(owner_id, &cash));
    }

    Ok(cash)
}

/// Withdraws cash, then broadcasts.
pub async fn withdraw_cash(
    pool: &DbPool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
    currency: &str,
    amount: &str,
    executed_at: Option<DateTime<Utc>>,
) -> Result<WithdrawOutcome, PortfolioError> {
    let currency =
        normalise_currency(currency).map_err(|e| PortfolioError::Invalid(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let amount_minor =
        parse_amount(amount, exponent).map_err(|e| PortfolioError::Invalid(e.to_string()))?;

    let outcome = cash_repo::withdraw_cash_at(pool, owner_id, &currency, amount_minor, executed_at)
        .await
        .map_err(PortfolioError::Db)?;

    if let WithdrawOutcome::Ok(ref cash) = outcome {
        if let Some(b) = broadcaster {
            b.send(cash_event(owner_id, cash));
        }
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn setup() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    fn trade_req<'a>(
        symbol: &'a str,
        quantity: &'a str,
        price: &'a str,
        currency: &'a str,
    ) -> TradeRequest<'a> {
        TradeRequest {
            symbol,
            quantity,
            price,
            currency,
            note: None,
            executed_at: None,
        }
    }

    /// Every rejection `buy` can produce, in one place — this replaces
    /// per-transport validation tests. Each transport keeps only a smoke test
    /// that a rejection surfaces in its own format.
    #[tokio::test]
    async fn test_buy_rejection_parity_table() {
        let pool = setup().await;
        let over_limit_note = "x".repeat(MAX_NOTE_LEN + 1);

        struct Case<'a> {
            name: &'a str,
            input: TradeRequest<'a>,
            expect: fn(&PortfolioError) -> bool,
        }

        let cases = vec![
            Case {
                name: "note one character over the limit",
                input: TradeRequest {
                    note: Some(&over_limit_note),
                    ..trade_req("AAPL", "10", "150.00", "USD")
                },
                expect: |e| matches!(e, PortfolioError::NoteTooLong { .. }),
            },
            Case {
                name: "an unparseable symbol",
                input: trade_req("", "10", "150.00", "USD"),
                expect: |e| matches!(e, PortfolioError::Invalid(_)),
            },
            Case {
                name: "an unknown currency",
                input: trade_req("AAPL", "10", "150.00", "DOLLARZ"),
                expect: |e| matches!(e, PortfolioError::Invalid(_)),
            },
            Case {
                name: "a non-numeric quantity",
                input: trade_req("AAPL", "abc", "150.00", "USD"),
                expect: |e| matches!(e, PortfolioError::Invalid(_)),
            },
        ];

        for case in cases {
            let err = buy(&pool, None, "@alice:a", case.input)
                .await
                .expect_err(&format!("{} must be rejected", case.name));
            assert!(
                (case.expect)(&err),
                "{}: unexpected error variant {:?}",
                case.name,
                err
            );
        }
    }

    /// The bug this whole module exists to fix: a buy of a symbol not already
    /// held must announce a creation, and a later buy of the same symbol must
    /// announce an update — regardless of which transport asked.
    #[tokio::test]
    async fn test_a_first_buy_announces_a_creation_and_a_later_one_an_update() {
        let pool = setup().await;
        let broadcaster = EventBroadcaster::default();
        let mut rx = broadcaster.subscribe();
        cash_repo::deposit_cash(&pool, "@alice:a", "USD", 2, 10_000_000)
            .await
            .unwrap();

        buy(
            &pool,
            Some(&broadcaster),
            "@alice:a",
            trade_req("AAPL", "10", "150.00", "USD"),
        )
        .await
        .unwrap();
        let mut saw_created = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, SystemEvent::PortfolioHoldingCreated(_)) {
                saw_created = true;
            }
        }
        assert!(saw_created, "a first buy must announce a creation");

        buy(
            &pool,
            Some(&broadcaster),
            "@alice:a",
            trade_req("AAPL", "5", "160.00", "USD"),
        )
        .await
        .unwrap();
        let mut saw_updated = false;
        let mut saw_created_again = false;
        while let Ok(event) = rx.try_recv() {
            match event {
                SystemEvent::PortfolioHoldingUpdated(_) => saw_updated = true,
                SystemEvent::PortfolioHoldingCreated(_) => saw_created_again = true,
                _ => {}
            }
        }
        assert!(
            saw_updated,
            "a second buy of the same symbol must announce an update"
        );
        assert!(
            !saw_created_again,
            "a top-up must not announce a second creation"
        );
    }

    /// A blank note stores nothing rather than an empty string.
    ///
    /// The REST path collapsed whitespace to `None` and the other two did not,
    /// so the same blank field stored `NULL` or `''` depending on who sent it.
    /// Pinned because the first version of this service dropped the collapse
    /// and nothing would have noticed: both render as nothing.
    #[test]
    fn test_a_blank_note_is_stored_as_nothing() {
        assert_eq!(validate_note(None).unwrap(), None);
        assert_eq!(validate_note(Some("   ")).unwrap(), None);
        assert_eq!(validate_note(Some("")).unwrap(), None);
        assert_eq!(validate_note(Some(" kept ")).unwrap(), Some(" kept "));

        let too_long = "x".repeat(MAX_NOTE_LEN + 1);
        assert!(matches!(
            validate_note(Some(&too_long)),
            Err(PortfolioError::NoteTooLong { .. })
        ));
    }
}
