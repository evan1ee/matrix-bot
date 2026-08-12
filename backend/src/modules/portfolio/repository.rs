use anyhow::Result;
use sqlx::SqlitePool;

use crate::modules::portfolio::models::{
    cost_of, realized_pl_minor, weighted_avg_cost_minor, CashBalance, Holding,
    PortfolioCurrencySummary,
};

const HOLDING_COLUMNS: &str = "id, owner_id, symbol, quantity_minor, avg_cost_minor, currency, \
                                currency_exponent, current_price_minor, price_updated_at, note, \
                                created_at, updated_at";
const CASH_COLUMNS: &str = "id, owner_id, currency, currency_exponent, balance_minor, updated_at";

/// The result of a buy. A second currency for a symbol already held is rejected
/// rather than guessed at — exactly like `finance` never sums two currencies
/// together, there is no rate source to reconcile a position priced in two of
/// them. A buy that would take cash negative is rejected the same way.
// 272 bytes in the success variant against a `String` in the rejections.
// Boxing would shrink the enum and complicate every `match` that reads a
// holding back — and this is returned once per `!portfolio buy`, on a path that
// has just done several database round trips. The stack bytes are not the cost
// that matters here.
#[allow(clippy::large_enum_variant)]
pub enum BuyOutcome {
    Ok { holding: Holding, cash: CashBalance },
    CurrencyMismatch { existing_currency: String },
    InsufficientCash { available_minor: i64, needed_minor: i64 },
}

/// The result of a sell.
pub enum SellOutcome {
    NotFound,
    /// Attempted to sell more than is held.
    ExceedsHolding { held_minor: i64 },
    /// The sale exactly closed the position; the row is gone.
    Closed { realized_pl_minor: i64, cash: CashBalance },
    /// The position was reduced but remains open.
    Reduced { holding: Holding, realized_pl_minor: i64, cash: CashBalance },
}

/// The result of a manual cash withdrawal. Also reused internally by [`PortfolioRepository::buy`]
/// as its funding gate: the "is there enough?" check and the debit happen in
/// this one function, so there is no separate check-then-act window for a buy
/// to race against.
pub enum WithdrawOutcome {
    Ok(CashBalance),
    InsufficientFunds { available_minor: i64 },
}

pub struct PortfolioRepository;

impl PortfolioRepository {
    pub async fn find(pool: &SqlitePool, owner_id: &str, symbol: &str) -> Result<Option<Holding>> {
        let holding = sqlx::query_as::<_, Holding>(&format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
        ))
        .bind(owner_id)
        .bind(symbol)
        .fetch_optional(pool)
        .await?;

        Ok(holding)
    }

    /// Looks up a holding the caller owns, by numeric id — the REST layer's key.
    pub async fn find_owned(pool: &SqlitePool, id: i64, owner_id: &str) -> Result<Option<Holding>> {
        let holding = sqlx::query_as::<_, Holding>(&format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ? AND owner_id = ?"
        ))
        .bind(id)
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        Ok(holding)
    }

    pub async fn list(pool: &SqlitePool, owner_id: &str) -> Result<Vec<Holding>> {
        let holdings = sqlx::query_as::<_, Holding>(&format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? ORDER BY symbol ASC"
        ))
        .bind(owner_id)
        .fetch_all(pool)
        .await?;

        Ok(holdings)
    }

    // ---- cash ------------------------------------------------------------

    /// The caller's cash balance in one currency, or a zero balance if they
    /// have never deposited into it — the same "no row until you touch it"
    /// convention as `settings::UserSettings`.
    pub async fn get_cash_or_zero(
        pool: &SqlitePool,
        owner_id: &str,
        currency: &str,
        currency_exponent: i64,
    ) -> Result<CashBalance> {
        let found = sqlx::query_as::<_, CashBalance>(&format!(
            "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?"
        ))
        .bind(owner_id)
        .bind(currency)
        .fetch_optional(pool)
        .await?;

        Ok(found.unwrap_or(CashBalance {
            id: 0,
            owner_id: owner_id.to_string(),
            currency: currency.to_string(),
            currency_exponent,
            balance_minor: 0,
            updated_at: None,
        }))
    }

    pub async fn list_cash(pool: &SqlitePool, owner_id: &str) -> Result<Vec<CashBalance>> {
        let balances = sqlx::query_as::<_, CashBalance>(&format!(
            "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE owner_id = ? ORDER BY currency ASC"
        ))
        .bind(owner_id)
        .fetch_all(pool)
        .await?;

        Ok(balances)
    }

    /// Adds to a cash balance, creating the row at zero first if needed. Never
    /// fails on the amount — a deposit can only ever increase how much you have.
    pub async fn deposit_cash(
        pool: &SqlitePool,
        owner_id: &str,
        currency: &str,
        currency_exponent: i64,
        amount_minor: i64,
    ) -> Result<CashBalance> {
        sqlx::query(
            "INSERT INTO portfolio_cash_balances (owner_id, currency, currency_exponent, balance_minor, updated_at) \
             VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT(owner_id, currency) DO UPDATE SET \
                balance_minor = balance_minor + excluded.balance_minor, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(owner_id)
        .bind(currency)
        .bind(currency_exponent)
        .bind(amount_minor)
        .execute(pool)
        .await?;

        Self::get_cash_or_zero(pool, owner_id, currency, currency_exponent).await
    }

    /// Subtracts from a cash balance. Rejected — with nothing written — if it
    /// would take the balance negative, including when there is no row at all
    /// (a balance of zero can never cover a positive withdrawal).
    pub async fn withdraw_cash(
        pool: &SqlitePool,
        owner_id: &str,
        currency: &str,
        amount_minor: i64,
    ) -> Result<WithdrawOutcome> {
        let existing: Option<(i64, i64)> = sqlx::query_as(
            "SELECT id, balance_minor FROM portfolio_cash_balances WHERE owner_id = ? AND currency = ?",
        )
        .bind(owner_id)
        .bind(currency)
        .fetch_optional(pool)
        .await?;

        let (id, balance) = match existing {
            Some(row) => row,
            None => return Ok(WithdrawOutcome::InsufficientFunds { available_minor: 0 }),
        };

        if amount_minor > balance {
            return Ok(WithdrawOutcome::InsufficientFunds { available_minor: balance });
        }

        sqlx::query(
            "UPDATE portfolio_cash_balances SET balance_minor = balance_minor - ?, updated_at = CURRENT_TIMESTAMP \
             WHERE id = ?",
        )
        .bind(amount_minor)
        .bind(id)
        .execute(pool)
        .await?;

        let updated = sqlx::query_as::<_, CashBalance>(&format!(
            "SELECT {CASH_COLUMNS} FROM portfolio_cash_balances WHERE id = ?"
        ))
        .bind(id)
        .fetch_one(pool)
        .await?;

        Ok(WithdrawOutcome::Ok(updated))
    }

    // ---- holdings ----------------------------------------------------------

    /// Opens a new position, or adds to an existing one with the quantity
    /// weighted into the average cost. Requires and debits sufficient cash in
    /// the buy's currency first — `withdraw_cash` is the funding gate, so the
    /// sufficiency check and the debit happen together with no window for a
    /// concurrent buy to race between them. Nothing is written, on either
    /// table, if the funds are not there.
    ///
    /// `note`, when given, replaces the stored note; when absent, the existing
    /// note (if any) is left untouched — a routine top-up must not silently
    /// wipe a note the user wrote earlier.
    #[allow(clippy::too_many_arguments)]
    pub async fn buy(
        pool: &SqlitePool,
        owner_id: &str,
        symbol: &str,
        quantity_minor: i64,
        price_minor: i64,
        currency: &str,
        currency_exponent: i64,
        note: Option<&str>,
    ) -> Result<BuyOutcome> {
        let existing = Self::find(pool, owner_id, symbol).await?;

        if let Some(h) = &existing {
            if h.currency != currency {
                return Ok(BuyOutcome::CurrencyMismatch { existing_currency: h.currency.clone() });
            }
        }

        let needed = cost_of(quantity_minor, price_minor);
        let cash = match Self::withdraw_cash(pool, owner_id, currency, needed).await? {
            WithdrawOutcome::Ok(cash) => cash,
            WithdrawOutcome::InsufficientFunds { available_minor } => {
                return Ok(BuyOutcome::InsufficientCash { available_minor, needed_minor: needed });
            }
        };

        let Some(existing) = existing else {
            sqlx::query(
                "INSERT INTO portfolio_holdings \
                 (owner_id, symbol, quantity_minor, avg_cost_minor, currency, currency_exponent, \
                  current_price_minor, price_updated_at, note) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL, ?)",
            )
            .bind(owner_id)
            .bind(symbol)
            .bind(quantity_minor)
            .bind(price_minor)
            .bind(currency)
            .bind(currency_exponent)
            .bind(note)
            .execute(pool)
            .await?;

            let holding = Self::find(pool, owner_id, symbol)
                .await?
                .ok_or_else(|| anyhow::anyhow!("inserted holding vanished before it could be read"))?;
            return Ok(BuyOutcome::Ok { holding, cash });
        };

        let new_quantity = existing.quantity_minor + quantity_minor;
        let new_avg_cost = weighted_avg_cost_minor(
            existing.quantity_minor,
            existing.avg_cost_minor,
            quantity_minor,
            price_minor,
        );

        if let Some(text) = note {
            sqlx::query(
                "UPDATE portfolio_holdings \
                 SET quantity_minor = ?, avg_cost_minor = ?, note = ?, updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?",
            )
            .bind(new_quantity)
            .bind(new_avg_cost)
            .bind(text)
            .bind(existing.id)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "UPDATE portfolio_holdings \
                 SET quantity_minor = ?, avg_cost_minor = ?, updated_at = CURRENT_TIMESTAMP \
                 WHERE id = ?",
            )
            .bind(new_quantity)
            .bind(new_avg_cost)
            .bind(existing.id)
            .execute(pool)
            .await?;
        }

        let holding = Self::find(pool, owner_id, symbol)
            .await?
            .ok_or_else(|| anyhow::anyhow!("updated holding vanished before it could be read"))?;
        Ok(BuyOutcome::Ok { holding, cash })
    }

    /// Reduces or closes a position, crediting cash with the sale's gross
    /// proceeds (quantity × sale price) — not just the realized gain. A
    /// losing sale still returns the principal that came back out; the P&L is
    /// reported separately for the reply, not what lands in cash.
    ///
    /// Selling the full quantity held deletes the row — average cost has no
    /// meaning for a position of zero.
    pub async fn sell(
        pool: &SqlitePool,
        owner_id: &str,
        symbol: &str,
        quantity_minor: i64,
        sell_price_minor: i64,
    ) -> Result<SellOutcome> {
        let Some(existing) = Self::find(pool, owner_id, symbol).await? else {
            return Ok(SellOutcome::NotFound);
        };

        if quantity_minor > existing.quantity_minor {
            return Ok(SellOutcome::ExceedsHolding { held_minor: existing.quantity_minor });
        }

        let realized = realized_pl_minor(quantity_minor, sell_price_minor, existing.avg_cost_minor);
        let proceeds = cost_of(quantity_minor, sell_price_minor);
        let cash =
            Self::deposit_cash(pool, owner_id, &existing.currency, existing.currency_exponent, proceeds)
                .await?;

        let remaining = existing.quantity_minor - quantity_minor;
        if remaining == 0 {
            sqlx::query("DELETE FROM portfolio_holdings WHERE id = ?")
                .bind(existing.id)
                .execute(pool)
                .await?;
            return Ok(SellOutcome::Closed { realized_pl_minor: realized, cash });
        }

        sqlx::query(
            "UPDATE portfolio_holdings SET quantity_minor = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(remaining)
        .bind(existing.id)
        .execute(pool)
        .await?;

        let holding = Self::find(pool, owner_id, symbol)
            .await?
            .ok_or_else(|| anyhow::anyhow!("reduced holding vanished before it could be read"))?;
        Ok(SellOutcome::Reduced { holding, realized_pl_minor: realized, cash })
    }

    pub async fn set_price(
        pool: &SqlitePool,
        owner_id: &str,
        symbol: &str,
        price_minor: i64,
    ) -> Result<Option<Holding>> {
        let result = sqlx::query(
            "UPDATE portfolio_holdings \
             SET current_price_minor = ?, price_updated_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
             WHERE owner_id = ? AND symbol = ?",
        )
        .bind(price_minor)
        .bind(owner_id)
        .bind(symbol)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }
        Self::find(pool, owner_id, symbol).await
    }

    pub async fn set_note(
        pool: &SqlitePool,
        owner_id: &str,
        symbol: &str,
        note: Option<&str>,
    ) -> Result<Option<Holding>> {
        let result = sqlx::query(
            "UPDATE portfolio_holdings SET note = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE owner_id = ? AND symbol = ?",
        )
        .bind(note)
        .bind(owner_id)
        .bind(symbol)
        .execute(pool)
        .await?;

        if result.rows_affected() == 0 {
            return Ok(None);
        }
        Self::find(pool, owner_id, symbol).await
    }

    /// Removes a holding entirely — an administrative correction (a typo'd
    /// symbol, a test entry), not a sale. Deliberately does not touch cash:
    /// only `buy` and `sell`, which represent an actual money-for-shares
    /// exchange, do that.
    pub async fn delete(pool: &SqlitePool, owner_id: &str, symbol: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?")
            .bind(owner_id)
            .bind(symbol)
            .execute(pool)
            .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Totals per currency across every holding and cash balance. Currencies
    /// are never combined. A currency seeded only by cash (a deposit with
    /// nothing bought yet) still appears, with zero holdings.
    pub async fn summary(pool: &SqlitePool, owner_id: &str) -> Result<Vec<PortfolioCurrencySummary>> {
        let holdings = Self::list(pool, owner_id).await?;
        let cash_balances = Self::list_cash(pool, owner_id).await?;

        let mut summaries: Vec<PortfolioCurrencySummary> = Vec::new();

        let entry_for = |summaries: &mut Vec<PortfolioCurrencySummary>, currency: &str, exponent: i64| {
            if !summaries.iter().any(|s| s.currency == currency) {
                summaries.push(PortfolioCurrencySummary {
                    currency: currency.to_string(),
                    currency_exponent: exponent,
                    cost_basis_minor: 0,
                    market_value_minor: 0,
                    unrealized_pl_minor: 0,
                    priced_holdings: 0,
                    unpriced_holdings: 0,
                    cash_minor: 0,
                });
            }
        };

        for holding in &holdings {
            entry_for(&mut summaries, &holding.currency, holding.currency_exponent);
            let entry = summaries.iter_mut().find(|s| s.currency == holding.currency).expect("just inserted");

            entry.cost_basis_minor += holding.cost_basis_minor();
            match holding.unrealized_pl_minor() {
                Some(pl) => {
                    entry.market_value_minor += holding.market_value_minor().unwrap_or(0);
                    entry.unrealized_pl_minor += pl;
                    entry.priced_holdings += 1;
                }
                None => entry.unpriced_holdings += 1,
            }
        }

        for cash in &cash_balances {
            entry_for(&mut summaries, &cash.currency, cash.currency_exponent);
            let entry = summaries.iter_mut().find(|s| s.currency == cash.currency).expect("just inserted");
            entry.cash_minor = cash.balance_minor;
        }

        summaries.sort_by(|a, b| a.currency.cmp(&b.currency));
        Ok(summaries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::modules::portfolio::models::QUANTITY_EXPONENT;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    fn qty(whole: i64) -> i64 {
        whole * 10i64.pow(QUANTITY_EXPONENT)
    }

    fn unwrap_ok(outcome: BuyOutcome) -> Holding {
        match outcome {
            BuyOutcome::Ok { holding, .. } => holding,
            BuyOutcome::CurrencyMismatch { existing_currency } => {
                panic!("expected Ok, got CurrencyMismatch({existing_currency})")
            }
            BuyOutcome::InsufficientCash { available_minor, needed_minor } => {
                panic!("expected Ok, got InsufficientCash(available={available_minor}, needed={needed_minor})")
            }
        }
    }

    /// Every buy test needs cash on hand first now that buying is gated on it;
    /// this deposits comfortably more than any single test spends.
    async fn fund(pool: &SqlitePool, owner_id: &str, currency: &str) {
        PortfolioRepository::deposit_cash(pool, owner_id, currency, 2, 100_000_000)
            .await
            .unwrap();
    }

    async fn fund_jpy(pool: &SqlitePool, owner_id: &str) {
        PortfolioRepository::deposit_cash(pool, owner_id, "JPY", 0, 100_000_000)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_buy_opens_a_new_position() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;

        let holding = unwrap_ok(
            PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
                .await
                .unwrap(),
        );

        assert_eq!(holding.quantity_minor, qty(10));
        assert_eq!(holding.avg_cost_minor, 15000);
        assert_eq!(holding.currency, "USD");
        assert!(holding.current_price_minor.is_none());
    }

    /// The weighted-average math is unit-tested in `models.rs`; this proves the
    /// repository actually applies it across two writes.
    #[tokio::test]
    async fn test_buying_more_blends_into_the_average_cost() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        let holding = unwrap_ok(
            PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(5), 18000, "USD", 2, None)
                .await
                .unwrap(),
        );

        assert_eq!(holding.quantity_minor, qty(15));
        assert_eq!(holding.avg_cost_minor, 16000, "expected $160.00 average");

        // Still exactly one row for this symbol, not two.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_holdings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    /// A second currency for the same symbol has no rate source to reconcile
    /// against the first — it must be rejected, not silently averaged.
    #[tokio::test]
    async fn test_buying_in_a_different_currency_is_rejected() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        fund(&pool, "@a:x", "EUR").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(5), 18000, "EUR", 2, None)
            .await
            .unwrap()
        {
            BuyOutcome::CurrencyMismatch { existing_currency } => {
                assert_eq!(existing_currency, "USD");
            }
            _ => panic!("a currency mismatch must not silently succeed"),
        }

        // Nothing was written by the rejected buy, including no EUR debited.
        let holding = PortfolioRepository::find(&pool, "@a:x", "AAPL").await.unwrap().unwrap();
        assert_eq!(holding.quantity_minor, qty(10));
        let eur_cash = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "EUR", 2).await.unwrap();
        assert_eq!(eur_cash.balance_minor, 100_000_000);
    }

    /// A top-up with no note must not erase a note written on an earlier buy.
    #[tokio::test]
    async fn test_buy_preserves_an_existing_note_when_none_is_given() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, Some("core position"))
            .await
            .unwrap();

        let holding = unwrap_ok(
            PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(5), 18000, "USD", 2, None)
                .await
                .unwrap(),
        );
        assert_eq!(holding.note.as_deref(), Some("core position"));

        let holding = unwrap_ok(
            PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(1), 20000, "USD", 2, Some("adding more"))
                .await
                .unwrap(),
        );
        assert_eq!(holding.note.as_deref(), Some("adding more"), "an explicit note must replace it");
    }

    #[tokio::test]
    async fn test_sell_reduces_an_open_position() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match PortfolioRepository::sell(&pool, "@a:x", "AAPL", qty(4), 18000).await.unwrap() {
            SellOutcome::Reduced { holding, realized_pl_minor, cash } => {
                assert_eq!(holding.quantity_minor, qty(6));
                assert_eq!(holding.avg_cost_minor, 15000, "avg cost of the remainder is unchanged");
                assert_eq!(realized_pl_minor, 12000, "$30 gain × 4 shares = $120.00");
                // Cash: started with 100_000_000 - 150000 (cost of 10@150) + 72000 (proceeds of 4@180).
                assert_eq!(cash.balance_minor, 100_000_000 - 150000 + 72000);
            }
            _ => panic!("expected Reduced, got a different outcome"),
        }
    }

    #[tokio::test]
    async fn test_selling_the_full_quantity_closes_the_position() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match PortfolioRepository::sell(&pool, "@a:x", "AAPL", qty(10), 18000).await.unwrap() {
            SellOutcome::Closed { realized_pl_minor, cash } => {
                assert_eq!(realized_pl_minor, 30000, "$30 gain × 10 shares = $300.00");
                // Full principal (150000) plus the gain (30000) returned to cash.
                assert_eq!(cash.balance_minor, 100_000_000 - 150000 + 180000);
            }
            _ => panic!("expected Closed, got a different outcome"),
        }

        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPL").await.unwrap().is_none());
    }

    /// A losing sale still returns the principal that came back out — cash
    /// must reflect gross proceeds, not the (negative) realized P&L.
    #[tokio::test]
    async fn test_a_losing_sale_still_credits_the_proceeds_not_just_the_loss() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match PortfolioRepository::sell(&pool, "@a:x", "AAPL", qty(10), 12000).await.unwrap() {
            SellOutcome::Closed { realized_pl_minor, cash } => {
                assert_eq!(realized_pl_minor, -30000, "sold at a $300.00 loss");
                // Proceeds are 10 @ $120.00 = $1200.00, credited in full even
                // though it's less than the $1500.00 that was spent.
                assert_eq!(cash.balance_minor, 100_000_000 - 150000 + 120000);
            }
            _ => panic!("expected Closed, got a different outcome"),
        }
    }

    #[tokio::test]
    async fn test_selling_more_than_is_held_is_rejected() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match PortfolioRepository::sell(&pool, "@a:x", "AAPL", qty(11), 18000).await.unwrap() {
            SellOutcome::ExceedsHolding { held_minor } => assert_eq!(held_minor, qty(10)),
            _ => panic!("expected ExceedsHolding, got a different outcome"),
        }

        // Nothing changed, including cash.
        let holding = PortfolioRepository::find(&pool, "@a:x", "AAPL").await.unwrap().unwrap();
        assert_eq!(holding.quantity_minor, qty(10));
        let cash = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "USD", 2).await.unwrap();
        assert_eq!(cash.balance_minor, 100_000_000 - 150000);
    }

    #[tokio::test]
    async fn test_selling_a_symbol_never_held_is_not_found() {
        let pool = pool().await;
        assert!(matches!(
            PortfolioRepository::sell(&pool, "@a:x", "AAPL", qty(1), 18000).await.unwrap(),
            SellOutcome::NotFound
        ));
    }

    #[tokio::test]
    async fn test_set_price_and_delete() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        let holding = PortfolioRepository::set_price(&pool, "@a:x", "AAPL", 18000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(holding.current_price_minor, Some(18000));

        assert!(PortfolioRepository::delete(&pool, "@a:x", "AAPL").await.unwrap());
        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPL").await.unwrap().is_none());
    }

    /// `delete` is an administrative correction, not a sale — it must not
    /// refund or otherwise touch cash.
    #[tokio::test]
    async fn test_delete_does_not_touch_cash() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();
        let before = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "USD", 2).await.unwrap();

        PortfolioRepository::delete(&pool, "@a:x", "AAPL").await.unwrap();

        let after = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "USD", 2).await.unwrap();
        assert_eq!(after.balance_minor, before.balance_minor);
    }

    #[tokio::test]
    async fn test_set_price_on_an_unknown_symbol_returns_none() {
        let pool = pool().await;
        assert!(PortfolioRepository::set_price(&pool, "@a:x", "AAPL", 18000).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_set_note_can_clear_it() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, Some("note"))
            .await
            .unwrap();

        let cleared = PortfolioRepository::set_note(&pool, "@a:x", "AAPL", None).await.unwrap().unwrap();
        assert!(cleared.note.is_none());
    }

    /// Holdings must be isolated per owner, exactly like every other owned table.
    #[tokio::test]
    async fn test_holdings_are_isolated_per_owner() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        fund(&pool, "@b:x", "USD").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();
        PortfolioRepository::buy(&pool, "@b:x", "AAPL", qty(5), 20000, "USD", 2, None)
            .await
            .unwrap();

        let mine = PortfolioRepository::list(&pool, "@a:x").await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].quantity_minor, qty(10));

        assert!(!PortfolioRepository::delete(&pool, "@b:x", "NONEXISTENT").await.unwrap());
    }

    #[tokio::test]
    async fn test_summary_groups_by_currency_and_separates_priced_from_unpriced() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        fund_jpy(&pool, "@a:x").await;
        PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();
        PortfolioRepository::buy(&pool, "@a:x", "MSFT", qty(2), 30000, "USD", 2, None)
            .await
            .unwrap();
        PortfolioRepository::buy(&pool, "@a:x", "SONY", qty(100), 2000, "JPY", 0, None)
            .await
            .unwrap();
        PortfolioRepository::set_price(&pool, "@a:x", "AAPL", 18000).await.unwrap();

        let summaries = PortfolioRepository::summary(&pool, "@a:x").await.unwrap();
        assert_eq!(summaries.len(), 2);

        let usd = summaries.iter().find(|s| s.currency == "USD").unwrap();
        assert_eq!(usd.cost_basis_minor, 150000 + 60000, "AAPL $1500 + MSFT $600");
        assert_eq!(usd.market_value_minor, 180000, "only AAPL has a price");
        assert_eq!(usd.unrealized_pl_minor, 30000);
        assert_eq!(usd.priced_holdings, 1);
        assert_eq!(usd.unpriced_holdings, 1);
        assert_eq!(usd.cash_minor, 100_000_000 - 150000 - 60000);

        let jpy = summaries.iter().find(|s| s.currency == "JPY").unwrap();
        assert_eq!(jpy.unpriced_holdings, 1);
        assert_eq!(jpy.priced_holdings, 0);
        assert_eq!(jpy.cash_minor, 100_000_000 - 200000);
    }

    /// A currency with a deposit but nothing bought yet must still be visible.
    #[tokio::test]
    async fn test_summary_includes_a_cash_only_currency() {
        let pool = pool().await;
        PortfolioRepository::deposit_cash(&pool, "@a:x", "GBP", 2, 50000).await.unwrap();

        let summaries = PortfolioRepository::summary(&pool, "@a:x").await.unwrap();
        assert_eq!(summaries.len(), 1);
        let gbp = &summaries[0];
        assert_eq!(gbp.currency, "GBP");
        assert_eq!(gbp.cash_minor, 50000);
        assert_eq!(gbp.cost_basis_minor, 0);
        assert_eq!(gbp.priced_holdings, 0);
        assert_eq!(gbp.unpriced_holdings, 0);
    }

    /// `find_owned` is what the REST layer uses to translate a numeric id into
    /// a symbol without leaking another owner's row.
    #[tokio::test]
    async fn test_find_owned_respects_ownership() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        let holding = unwrap_ok(
            PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
                .await
                .unwrap(),
        );

        assert!(PortfolioRepository::find_owned(&pool, holding.id, "@a:x").await.unwrap().is_some());
        assert!(PortfolioRepository::find_owned(&pool, holding.id, "@b:x").await.unwrap().is_none());
    }

    // ---- cash ----------------------------------------------------------

    #[tokio::test]
    async fn test_a_user_with_no_deposit_has_a_zero_balance() {
        let pool = pool().await;
        let cash = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "USD", 2).await.unwrap();
        assert_eq!(cash.balance_minor, 0);
        assert_eq!(cash.currency, "USD");
    }

    #[tokio::test]
    async fn test_deposit_creates_then_adds() {
        let pool = pool().await;
        let first = PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 100000).await.unwrap();
        assert_eq!(first.balance_minor, 100000);

        let second = PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 50000).await.unwrap();
        assert_eq!(second.balance_minor, 150000);

        // Still exactly one row for this owner+currency.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM portfolio_cash_balances")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_withdraw_reduces_the_balance() {
        let pool = pool().await;
        PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 100000).await.unwrap();

        match PortfolioRepository::withdraw_cash(&pool, "@a:x", "USD", 40000).await.unwrap() {
            WithdrawOutcome::Ok(cash) => assert_eq!(cash.balance_minor, 60000),
            WithdrawOutcome::InsufficientFunds { .. } => panic!("expected Ok"),
        }
    }

    /// Withdrawing more than is held is rejected and changes nothing — with no
    /// row at all, that means treating the balance as zero, not erroring.
    #[tokio::test]
    async fn test_withdraw_exceeding_the_balance_is_rejected() {
        let pool = pool().await;
        PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 10000).await.unwrap();

        match PortfolioRepository::withdraw_cash(&pool, "@a:x", "USD", 10001).await.unwrap() {
            WithdrawOutcome::InsufficientFunds { available_minor } => assert_eq!(available_minor, 10000),
            WithdrawOutcome::Ok(_) => panic!("expected InsufficientFunds"),
        }
        let cash = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "USD", 2).await.unwrap();
        assert_eq!(cash.balance_minor, 10000, "the rejected withdrawal must not change the balance");

        match PortfolioRepository::withdraw_cash(&pool, "@a:x", "EUR", 1).await.unwrap() {
            WithdrawOutcome::InsufficientFunds { available_minor } => assert_eq!(available_minor, 0),
            WithdrawOutcome::Ok(_) => panic!("a currency never deposited into has nothing to withdraw"),
        }
    }

    /// Buying without enough cash must write nothing — no holding, no cash
    /// change — and say exactly how much was needed versus available.
    #[tokio::test]
    async fn test_buy_without_enough_cash_is_rejected_and_writes_nothing() {
        let pool = pool().await;
        PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 100000).await.unwrap();

        match PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap()
        {
            BuyOutcome::InsufficientCash { available_minor, needed_minor } => {
                assert_eq!(available_minor, 100000);
                assert_eq!(needed_minor, 150000);
            }
            _ => panic!("expected InsufficientCash"),
        }

        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPL").await.unwrap().is_none());
        let cash = PortfolioRepository::get_cash_or_zero(&pool, "@a:x", "USD", 2).await.unwrap();
        assert_eq!(cash.balance_minor, 100000, "a rejected buy must not touch cash");
    }

    /// A buy at exactly the available balance must succeed, leaving zero.
    #[tokio::test]
    async fn test_buy_spending_exactly_all_available_cash_succeeds() {
        let pool = pool().await;
        PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 150000).await.unwrap();

        let BuyOutcome::Ok { cash, .. } =
            PortfolioRepository::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
                .await
                .unwrap()
        else {
            panic!("expected Ok");
        };
        assert_eq!(cash.balance_minor, 0);
    }

    #[tokio::test]
    async fn test_cash_is_isolated_per_owner_and_per_currency() {
        let pool = pool().await;
        PortfolioRepository::deposit_cash(&pool, "@a:x", "USD", 2, 100000).await.unwrap();
        PortfolioRepository::deposit_cash(&pool, "@a:x", "EUR", 2, 5000).await.unwrap();
        PortfolioRepository::deposit_cash(&pool, "@b:x", "USD", 2, 999999).await.unwrap();

        let mine = PortfolioRepository::list_cash(&pool, "@a:x").await.unwrap();
        assert_eq!(mine.len(), 2);
        assert_eq!(mine.iter().find(|c| c.currency == "USD").unwrap().balance_minor, 100000);
        assert_eq!(mine.iter().find(|c| c.currency == "EUR").unwrap().balance_minor, 5000);

        let theirs = PortfolioRepository::list_cash(&pool, "@b:x").await.unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].balance_minor, 999999);
    }
}
