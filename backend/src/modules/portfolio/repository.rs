use crate::db::DbPool;
use anyhow::Result;

use super::cash_repo;
use crate::modules::portfolio::models::{Holding, PortfolioCurrencySummary};

const HOLDING_COLUMNS: &str = "id, owner_id, symbol, quantity_minor, avg_cost_minor, currency, \
                                currency_exponent, current_price_minor, price_updated_at, note, \
                                created_at, updated_at";

pub use crate::modules::portfolio::models::{BuyOutcome, SellOutcome, WithdrawOutcome};

/// Reading and editing holdings.
///
/// Buying, selling and cash movement are not here: they are transactional and
/// live in [`super::trade`] and [`super::cash_repo`] with the audit-log writes
/// they have to stay consistent with. This struct used to forward to both,
/// which meant every operation was written twice and callers picked a level at
/// random.
pub struct PortfolioRepository;

impl PortfolioRepository {
    pub async fn find(pool: &DbPool, owner_id: &str, symbol: &str) -> Result<Option<Holding>> {
        let holding = crate::db_fetch_optional!(
            pool,
            Holding,
            &format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?"
        ),
            owner_id,
            symbol
        )?;

        Ok(holding)
    }

    /// Looks up a holding the caller owns, by numeric id — the REST layer's key.
    pub async fn find_owned(pool: &DbPool, id: i64, owner_id: &str) -> Result<Option<Holding>> {
        let holding = crate::db_fetch_optional!(
            pool,
            Holding,
            &format!(
                "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE id = ? AND owner_id = ?"
            ),
            id,
            owner_id
        )?;

        Ok(holding)
    }

    pub async fn list(pool: &DbPool, owner_id: &str) -> Result<Vec<Holding>> {
        let holdings = crate::db_fetch_all!(pool, Holding, &format!(
            "SELECT {HOLDING_COLUMNS} FROM portfolio_holdings WHERE owner_id = ? ORDER BY symbol ASC"
        ), owner_id)?;

        Ok(holdings)
    }

    // ---- holdings ----------------------------------------------------------

    pub async fn set_price(
        pool: &DbPool,
        owner_id: &str,
        symbol: &str,
        price_minor: i64,
    ) -> Result<Option<Holding>> {
        let result = crate::db_execute!(pool, "UPDATE portfolio_holdings \
             SET current_price_minor = ?, price_updated_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP \
             WHERE owner_id = ? AND symbol = ?", price_minor, owner_id, symbol)?;

        if result == 0 {
            return Ok(None);
        }
        Self::find(pool, owner_id, symbol).await
    }

    pub async fn set_note(
        pool: &DbPool,
        owner_id: &str,
        symbol: &str,
        note: Option<&str>,
    ) -> Result<Option<Holding>> {
        let result = crate::db_execute!(
            pool,
            "UPDATE portfolio_holdings SET note = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE owner_id = ? AND symbol = ?",
            note,
            owner_id,
            symbol
        )?;

        if result == 0 {
            return Ok(None);
        }
        Self::find(pool, owner_id, symbol).await
    }

    /// Renames a holding's ticker symbol. Rejects duplicates if the target symbol
    /// is already owned by this user under a different holding row.
    pub async fn update_symbol(
        pool: &DbPool,
        owner_id: &str,
        id: i64,
        new_symbol: &str,
    ) -> Result<Result<Holding, String>> {
        let normalized = match crate::modules::portfolio::models::normalise_symbol(new_symbol) {
            Ok(sym) => sym,
            Err(e) => return Ok(Err(e.to_string())),
        };

        if let Some(existing) = Self::find(pool, owner_id, &normalized).await? {
            if existing.id != id {
                return Ok(Err(format!("Position in {} already exists", normalized)));
            }
        }

        let result = crate::db_execute!(pool, "UPDATE portfolio_holdings SET symbol = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ? AND owner_id = ?", &normalized, id, owner_id)?;

        if result == 0 {
            return Err(anyhow::anyhow!("Holding not found or access denied"));
        }

        match Self::find_owned(pool, id, owner_id).await? {
            Some(h) => Ok(Ok(h)),
            None => Err(anyhow::anyhow!("Updated holding vanished")),
        }
    }

    /// Removes a holding entirely — an administrative correction (a typo'd
    /// symbol, a test entry), not a sale. Deliberately does not touch cash:
    /// only `buy` and `sell`, which represent an actual money-for-shares
    /// exchange, do that.
    pub async fn delete(pool: &DbPool, owner_id: &str, symbol: &str) -> Result<bool> {
        let result = crate::db_execute!(
            pool,
            "DELETE FROM portfolio_holdings WHERE owner_id = ? AND symbol = ?",
            owner_id,
            symbol
        )?;

        Ok(result > 0)
    }

    /// Totals per currency across every holding and cash balance. Currencies
    /// are never combined. A currency seeded only by cash (a deposit with
    /// nothing bought yet) still appears, with zero holdings.
    pub async fn summary(pool: &DbPool, owner_id: &str) -> Result<Vec<PortfolioCurrencySummary>> {
        let holdings = Self::list(pool, owner_id).await?;
        let cash_balances = cash_repo::list_cash(pool, owner_id).await?;
        Ok(crate::modules::portfolio::models::build_summaries(
            &holdings,
            &cash_balances,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::modules::portfolio::models::QUANTITY_EXPONENT;
    use crate::modules::portfolio::trade;
    use sqlx::SqlitePool;

    async fn pool() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
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
            BuyOutcome::InsufficientCash {
                available_minor,
                needed_minor,
            } => {
                panic!("expected Ok, got InsufficientCash(available={available_minor}, needed={needed_minor})")
            }
        }
    }

    /// Every buy test needs cash on hand first now that buying is gated on it;
    /// this deposits comfortably more than any single test spends.
    async fn fund(pool: &DbPool, owner_id: &str, currency: &str) {
        cash_repo::deposit_cash(pool, owner_id, currency, 2, 100_000_000)
            .await
            .unwrap();
    }

    async fn fund_jpy(pool: &DbPool, owner_id: &str) {
        cash_repo::deposit_cash(pool, owner_id, "JPY", 0, 100_000_000)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_buy_opens_a_new_position() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;

        let holding = unwrap_ok(
            trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
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
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        let holding = unwrap_ok(
            trade::buy(&pool, "@a:x", "AAPL", qty(5), 18000, "USD", 2, None)
                .await
                .unwrap(),
        );

        assert_eq!(holding.quantity_minor, qty(15));
        assert_eq!(holding.avg_cost_minor, 16000, "expected $160.00 average");

        // Still exactly one row for this symbol, not two.
        let count: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM portfolio_holdings").unwrap();
        assert_eq!(count, 1);
    }

    /// A second currency for the same symbol has no rate source to reconcile
    /// against the first — it must be rejected, not silently averaged.
    #[tokio::test]
    async fn test_buying_in_a_different_currency_is_rejected() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        fund(&pool, "@a:x", "EUR").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match trade::buy(&pool, "@a:x", "AAPL", qty(5), 18000, "EUR", 2, None)
            .await
            .unwrap()
        {
            BuyOutcome::CurrencyMismatch { existing_currency } => {
                assert_eq!(existing_currency, "USD");
            }
            _ => panic!("a currency mismatch must not silently succeed"),
        }

        // Nothing was written by the rejected buy, including no EUR debited.
        let holding = PortfolioRepository::find(&pool, "@a:x", "AAPL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(holding.quantity_minor, qty(10));
        let eur_cash = cash_repo::get_cash_or_zero(&pool, "@a:x", "EUR", 2)
            .await
            .unwrap();
        assert_eq!(eur_cash.balance_minor, 100_000_000);
    }

    /// A top-up with no note must not erase a note written on an earlier buy.
    #[tokio::test]
    async fn test_buy_preserves_an_existing_note_when_none_is_given() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        trade::buy(
            &pool,
            "@a:x",
            "AAPL",
            qty(10),
            15000,
            "USD",
            2,
            Some("core position"),
        )
        .await
        .unwrap();

        let holding = unwrap_ok(
            trade::buy(&pool, "@a:x", "AAPL", qty(5), 18000, "USD", 2, None)
                .await
                .unwrap(),
        );
        assert_eq!(holding.note.as_deref(), Some("core position"));

        let holding = unwrap_ok(
            trade::buy(
                &pool,
                "@a:x",
                "AAPL",
                qty(1),
                20000,
                "USD",
                2,
                Some("adding more"),
            )
            .await
            .unwrap(),
        );
        assert_eq!(
            holding.note.as_deref(),
            Some("adding more"),
            "an explicit note must replace it"
        );
    }

    #[tokio::test]
    async fn test_sell_reduces_an_open_position() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match trade::sell(&pool, "@a:x", "AAPL", qty(4), 18000)
            .await
            .unwrap()
        {
            SellOutcome::Reduced {
                holding,
                realized_pl_minor,
                cash,
            } => {
                assert_eq!(holding.quantity_minor, qty(6));
                assert_eq!(
                    holding.avg_cost_minor, 15000,
                    "avg cost of the remainder is unchanged"
                );
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
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match trade::sell(&pool, "@a:x", "AAPL", qty(10), 18000)
            .await
            .unwrap()
        {
            SellOutcome::Closed {
                realized_pl_minor,
                cash,
            } => {
                assert_eq!(realized_pl_minor, 30000, "$30 gain × 10 shares = $300.00");
                // Full principal (150000) plus the gain (30000) returned to cash.
                assert_eq!(cash.balance_minor, 100_000_000 - 150000 + 180000);
            }
            _ => panic!("expected Closed, got a different outcome"),
        }

        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPL")
            .await
            .unwrap()
            .is_none());
    }

    /// A losing sale still returns the principal that came back out — cash
    /// must reflect gross proceeds, not the (negative) realized P&L.
    #[tokio::test]
    async fn test_a_losing_sale_still_credits_the_proceeds_not_just_the_loss() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match trade::sell(&pool, "@a:x", "AAPL", qty(10), 12000)
            .await
            .unwrap()
        {
            SellOutcome::Closed {
                realized_pl_minor,
                cash,
            } => {
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
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        match trade::sell(&pool, "@a:x", "AAPL", qty(11), 18000)
            .await
            .unwrap()
        {
            SellOutcome::ExceedsHolding { held_minor } => assert_eq!(held_minor, qty(10)),
            _ => panic!("expected ExceedsHolding, got a different outcome"),
        }

        // Nothing changed, including cash.
        let holding = PortfolioRepository::find(&pool, "@a:x", "AAPL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(holding.quantity_minor, qty(10));
        let cash = cash_repo::get_cash_or_zero(&pool, "@a:x", "USD", 2)
            .await
            .unwrap();
        assert_eq!(cash.balance_minor, 100_000_000 - 150000);
    }

    #[tokio::test]
    async fn test_selling_a_symbol_never_held_is_not_found() {
        let pool = pool().await;
        assert!(matches!(
            trade::sell(&pool, "@a:x", "AAPL", qty(1), 18000)
                .await
                .unwrap(),
            SellOutcome::NotFound
        ));
    }

    #[tokio::test]
    async fn test_set_price_and_delete() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();

        let holding = PortfolioRepository::set_price(&pool, "@a:x", "AAPL", 18000)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(holding.current_price_minor, Some(18000));

        assert!(PortfolioRepository::delete(&pool, "@a:x", "AAPL")
            .await
            .unwrap());
        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPL")
            .await
            .unwrap()
            .is_none());
    }

    /// `delete` is an administrative correction, not a sale — it must not
    /// refund or otherwise touch cash.
    #[tokio::test]
    async fn test_delete_does_not_touch_cash() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();
        let before = cash_repo::get_cash_or_zero(&pool, "@a:x", "USD", 2)
            .await
            .unwrap();

        PortfolioRepository::delete(&pool, "@a:x", "AAPL")
            .await
            .unwrap();

        let after = cash_repo::get_cash_or_zero(&pool, "@a:x", "USD", 2)
            .await
            .unwrap();
        assert_eq!(after.balance_minor, before.balance_minor);
    }

    #[tokio::test]
    async fn test_set_price_on_an_unknown_symbol_returns_none() {
        let pool = pool().await;
        assert!(PortfolioRepository::set_price(&pool, "@a:x", "AAPL", 18000)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_set_note_can_clear_it() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        trade::buy(
            &pool,
            "@a:x",
            "AAPL",
            qty(10),
            15000,
            "USD",
            2,
            Some("note"),
        )
        .await
        .unwrap();

        let cleared = PortfolioRepository::set_note(&pool, "@a:x", "AAPL", None)
            .await
            .unwrap()
            .unwrap();
        assert!(cleared.note.is_none());
    }

    /// Holdings must be isolated per owner, exactly like every other owned table.
    #[tokio::test]
    async fn test_holdings_are_isolated_per_owner() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        fund(&pool, "@b:x", "USD").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();
        trade::buy(&pool, "@b:x", "AAPL", qty(5), 20000, "USD", 2, None)
            .await
            .unwrap();

        let mine = PortfolioRepository::list(&pool, "@a:x").await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].quantity_minor, qty(10));

        assert!(!PortfolioRepository::delete(&pool, "@b:x", "NONEXISTENT")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_summary_groups_by_currency_and_separates_priced_from_unpriced() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;
        fund_jpy(&pool, "@a:x").await;
        trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap();
        trade::buy(&pool, "@a:x", "MSFT", qty(2), 30000, "USD", 2, None)
            .await
            .unwrap();
        trade::buy(&pool, "@a:x", "SONY", qty(100), 2000, "JPY", 0, None)
            .await
            .unwrap();
        PortfolioRepository::set_price(&pool, "@a:x", "AAPL", 18000)
            .await
            .unwrap();

        let summaries = PortfolioRepository::summary(&pool, "@a:x").await.unwrap();
        assert_eq!(summaries.len(), 2);

        let usd = summaries.iter().find(|s| s.currency == "USD").unwrap();
        assert_eq!(
            usd.cost_basis_minor,
            150000 + 60000,
            "AAPL $1500 + MSFT $600"
        );
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
        cash_repo::deposit_cash(&pool, "@a:x", "GBP", 2, 50000)
            .await
            .unwrap();

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
            trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
                .await
                .unwrap(),
        );

        assert!(PortfolioRepository::find_owned(&pool, holding.id, "@a:x")
            .await
            .unwrap()
            .is_some());
        assert!(PortfolioRepository::find_owned(&pool, holding.id, "@b:x")
            .await
            .unwrap()
            .is_none());
    }

    // ---- cash ----------------------------------------------------------

    #[tokio::test]
    async fn test_a_user_with_no_deposit_has_a_zero_balance() {
        let pool = pool().await;
        let cash = cash_repo::get_cash_or_zero(&pool, "@a:x", "USD", 2)
            .await
            .unwrap();
        assert_eq!(cash.balance_minor, 0);
        assert_eq!(cash.currency, "USD");
    }

    #[tokio::test]
    async fn test_deposit_creates_then_adds() {
        let pool = pool().await;
        let first = cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 100000)
            .await
            .unwrap();
        assert_eq!(first.balance_minor, 100000);

        let second = cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 50000)
            .await
            .unwrap();
        assert_eq!(second.balance_minor, 150000);

        // Still exactly one row for this owner+currency.
        let count: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM portfolio_cash_balances")
                .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_withdraw_reduces_the_balance() {
        let pool = pool().await;
        cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 100000)
            .await
            .unwrap();

        match cash_repo::withdraw_cash(&pool, "@a:x", "USD", 40000)
            .await
            .unwrap()
        {
            WithdrawOutcome::Ok(cash) => assert_eq!(cash.balance_minor, 60000),
            WithdrawOutcome::InsufficientFunds { .. } => panic!("expected Ok"),
        }
    }

    /// Withdrawing more than is held is rejected and changes nothing — with no
    /// row at all, that means treating the balance as zero, not erroring.
    #[tokio::test]
    async fn test_withdraw_exceeding_the_balance_is_rejected() {
        let pool = pool().await;
        cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 10000)
            .await
            .unwrap();

        match cash_repo::withdraw_cash(&pool, "@a:x", "USD", 10001)
            .await
            .unwrap()
        {
            WithdrawOutcome::InsufficientFunds { available_minor } => {
                assert_eq!(available_minor, 10000)
            }
            WithdrawOutcome::Ok(_) => panic!("expected InsufficientFunds"),
        }
        let cash = cash_repo::get_cash_or_zero(&pool, "@a:x", "USD", 2)
            .await
            .unwrap();
        assert_eq!(
            cash.balance_minor, 10000,
            "the rejected withdrawal must not change the balance"
        );

        match cash_repo::withdraw_cash(&pool, "@a:x", "EUR", 1)
            .await
            .unwrap()
        {
            WithdrawOutcome::InsufficientFunds { available_minor } => {
                assert_eq!(available_minor, 0)
            }
            WithdrawOutcome::Ok(_) => {
                panic!("a currency never deposited into has nothing to withdraw")
            }
        }
    }

    /// Buying without enough cash must write nothing — no holding, no cash
    /// change — and say exactly how much was needed versus available.
    #[tokio::test]
    async fn test_buy_without_enough_cash_is_rejected_and_writes_nothing() {
        let pool = pool().await;
        cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 100000)
            .await
            .unwrap();

        match trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
            .await
            .unwrap()
        {
            BuyOutcome::InsufficientCash {
                available_minor,
                needed_minor,
            } => {
                assert_eq!(available_minor, 100000);
                assert_eq!(needed_minor, 150000);
            }
            _ => panic!("expected InsufficientCash"),
        }

        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPL")
            .await
            .unwrap()
            .is_none());
        let cash = cash_repo::get_cash_or_zero(&pool, "@a:x", "USD", 2)
            .await
            .unwrap();
        assert_eq!(
            cash.balance_minor, 100000,
            "a rejected buy must not touch cash"
        );
    }

    /// A buy at exactly the available balance must succeed, leaving zero.
    #[tokio::test]
    async fn test_buy_spending_exactly_all_available_cash_succeeds() {
        let pool = pool().await;
        cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 150000)
            .await
            .unwrap();

        let BuyOutcome::Ok { cash, .. } =
            trade::buy(&pool, "@a:x", "AAPL", qty(10), 15000, "USD", 2, None)
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
        cash_repo::deposit_cash(&pool, "@a:x", "USD", 2, 100000)
            .await
            .unwrap();
        cash_repo::deposit_cash(&pool, "@a:x", "EUR", 2, 5000)
            .await
            .unwrap();
        cash_repo::deposit_cash(&pool, "@b:x", "USD", 2, 999999)
            .await
            .unwrap();

        let mine = cash_repo::list_cash(&pool, "@a:x").await.unwrap();
        assert_eq!(mine.len(), 2);
        assert_eq!(
            mine.iter()
                .find(|c| c.currency == "USD")
                .unwrap()
                .balance_minor,
            100000
        );
        assert_eq!(
            mine.iter()
                .find(|c| c.currency == "EUR")
                .unwrap()
                .balance_minor,
            5000
        );

        let theirs = cash_repo::list_cash(&pool, "@b:x").await.unwrap();
        assert_eq!(theirs.len(), 1);
        assert_eq!(theirs[0].balance_minor, 999999);
    }

    #[tokio::test]
    async fn test_update_symbol() {
        let pool = pool().await;
        fund(&pool, "@a:x", "USD").await;

        let holding = unwrap_ok(
            trade::buy(&pool, "@a:x", "AAPLL", qty(10), 15000, "USD", 2, None)
                .await
                .unwrap(),
        );

        // Rename AAPLL -> AAPL
        let res = PortfolioRepository::update_symbol(&pool, "@a:x", holding.id, "aapl")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(res.symbol, "AAPL");

        // Verify lookup by new symbol works
        let found = PortfolioRepository::find(&pool, "@a:x", "AAPL")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, holding.id);
        assert!(PortfolioRepository::find(&pool, "@a:x", "AAPLL")
            .await
            .unwrap()
            .is_none());

        // Attempting to rename to an already held symbol must fail
        unwrap_ok(
            trade::buy(&pool, "@a:x", "MSFT", qty(5), 30000, "USD", 2, None)
                .await
                .unwrap(),
        );
        let conflict = PortfolioRepository::update_symbol(&pool, "@a:x", holding.id, "MSFT")
            .await
            .unwrap();
        assert!(conflict.is_err());
    }
}
