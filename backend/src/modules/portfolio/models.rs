use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Decimal places quantity is stored at — enough for a fractional share and for
/// satoshi-level crypto precision. Quantity goes through the same integer-minor-
/// unit technique as money (`finance::money`) for the same reason: no value can
/// be lost to binary float rounding on the way in or out.
pub const QUANTITY_EXPONENT: u32 = 8;

const MAX_SYMBOL_LEN: usize = 12;

/// Normalises a ticker/symbol: uppercase, 1–12 chars of letters, digits, `.` or
/// `-` (covers tickers like `BRK.B` alongside plain `AAPL`/`BTC`).
pub fn normalise_symbol(raw: &str) -> Result<String, String> {
    let symbol = raw.trim().to_uppercase();
    if symbol.is_empty() || symbol.len() > MAX_SYMBOL_LEN {
        return Err(format!("Symbol must be 1–{} characters", MAX_SYMBOL_LEN));
    }
    if !symbol.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
        return Err("Symbol must be letters, digits, '.' or '-' only".to_string());
    }
    Ok(symbol)
}

/// One investment position: symbol, quantity, and weighted-average cost basis.
///
/// `quantity_minor` and `avg_cost_minor` are always positive integers, scaled at
/// [`QUANTITY_EXPONENT`] and `currency_exponent` respectively. `current_price_minor`
/// is `None` until the user records one — an unset price is a normal state, not
/// an error, so market value and unrealized P&L are computed only when it is set.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct Holding {
    pub id: i64,
    /// The sole ownership key, matching `AuthUser::owner_id()`.
    pub owner_id: String,
    pub symbol: String,
    pub quantity_minor: i64,
    pub avg_cost_minor: i64,
    pub currency: String,
    /// Decimal places for this row's currency, stored so a later change to the
    /// currency table cannot rewrite what this row means.
    pub currency_exponent: i64,
    pub current_price_minor: Option<i64>,
    pub price_updated_at: Option<DateTime<Utc>>,
    /// A single freeform comment, most-recent-wins — not a transaction log.
    pub note: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl Holding {
    /// Cost basis for the full position: `quantity * avg_cost`.
    pub fn cost_basis_minor(&self) -> i64 {
        scale_product(self.quantity_minor, self.avg_cost_minor)
    }

    /// Current market value, or `None` while no price has been recorded.
    pub fn market_value_minor(&self) -> Option<i64> {
        self.current_price_minor.map(|price| scale_product(self.quantity_minor, price))
    }

    /// `market value - cost basis`, or `None` while no price has been recorded.
    /// May legitimately be negative.
    pub fn unrealized_pl_minor(&self) -> Option<i64> {
        self.market_value_minor().map(|value| value - self.cost_basis_minor())
    }
}

/// `quantity_minor * price_minor / 10^QUANTITY_EXPONENT`, in minor currency
/// units. Quantity's scale cancels against the divisor, leaving the result at
/// `currency_exponent` scale — the same trick [`weighted_avg_cost_minor`] uses.
/// `i128` intermediates: a large crypto quantity times a large price can
/// approach `i64`'s ceiling.
///
/// Also the public entry point for "what does this quantity at this price cost
/// in cash" — a buy's required debit and a sell's gross proceeds are both this
/// same computation, just read at different points in the trade.
pub(crate) fn scale_product(quantity_minor: i64, price_minor: i64) -> i64 {
    let product = quantity_minor as i128 * price_minor as i128;
    (product / 10i128.pow(QUANTITY_EXPONENT)) as i64
}

/// Cash cost (or proceeds) of `quantity_minor` units at `price_minor` each.
pub fn cost_of(quantity_minor: i64, price_minor: i64) -> i64 {
    scale_product(quantity_minor, price_minor)
}

/// The new weighted-average cost after adding `new_qty_minor` at `new_price_minor`
/// to an existing `existing_qty_minor` held at `existing_avg_cost_minor`.
///
/// `existing_qty_minor` may be zero (opening a new position), in which case the
/// result is simply `new_price_minor`.
pub fn weighted_avg_cost_minor(
    existing_qty_minor: i64,
    existing_avg_cost_minor: i64,
    new_qty_minor: i64,
    new_price_minor: i64,
) -> i64 {
    let existing_cost = existing_qty_minor as i128 * existing_avg_cost_minor as i128;
    let new_cost = new_qty_minor as i128 * new_price_minor as i128;
    let total_qty = existing_qty_minor as i128 + new_qty_minor as i128;

    ((existing_cost + new_cost) / total_qty) as i64
}

/// Realized profit or loss from selling `qty_sold_minor` units at `sell_price_minor`
/// out of a position whose cost basis is `avg_cost_minor`. May be negative.
pub fn realized_pl_minor(qty_sold_minor: i64, sell_price_minor: i64, avg_cost_minor: i64) -> i64 {
    let diff = sell_price_minor as i128 - avg_cost_minor as i128;
    ((qty_sold_minor as i128 * diff) / 10i128.pow(QUANTITY_EXPONENT)) as i64
}

/// Totals for one currency. Currencies are never combined — without a rate
/// source, adding a USD position to a JPY position produces a meaningless
/// number, exactly as `finance::models::CurrencySummary` documents.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PortfolioCurrencySummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub cost_basis_minor: i64,
    /// Market value of only the holdings that have a price set.
    pub market_value_minor: i64,
    /// Unrealized P&L of only the holdings that have a price set.
    pub unrealized_pl_minor: i64,
    pub priced_holdings: i64,
    pub unpriced_holdings: i64,
    /// Present even for a currency with no holdings at all — a deposit with
    /// nothing bought yet must still show up here.
    pub cash_minor: i64,
}

/// Cash available to spend on a buy, in one currency. A missing row means a
/// zero balance — the same "no row until you touch it" convention as
/// `settings::UserSettings`.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct CashBalance {
    pub id: i64,
    /// The sole ownership key, matching `AuthUser::owner_id()`.
    pub owner_id: String,
    pub currency: String,
    pub currency_exponent: i64,
    /// Never negative — enforced in `PortfolioRepository`, not the schema, the
    /// same app-level-only convention `quantity_minor`/`avg_cost_minor` use.
    pub balance_minor: i64,
    pub updated_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(qty: i64, avg_cost: i64, price: Option<i64>) -> Holding {
        Holding {
            id: 1,
            owner_id: "@a:x".to_string(),
            symbol: "AAPL".to_string(),
            quantity_minor: qty,
            avg_cost_minor: avg_cost,
            currency: "USD".to_string(),
            currency_exponent: 2,
            current_price_minor: price,
            price_updated_at: None,
            note: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn test_cost_of_is_quantity_times_price() {
        let qty = 10 * 10i64.pow(QUANTITY_EXPONENT);
        assert_eq!(cost_of(qty, 15000), 150000, "10 units @ $150.00 costs $1500.00");

        // The same function is reused for a sell's gross proceeds, not just a
        // buy's cost — the two are the same computation at different prices.
        assert_eq!(cost_of(qty, 18000), 180000, "10 units sold @ $180.00 returns $1800.00");
    }

    #[test]
    fn test_normalise_symbol() {
        assert_eq!(normalise_symbol("aapl"), Ok("AAPL".to_string()));
        assert_eq!(normalise_symbol(" btc "), Ok("BTC".to_string()));
        assert_eq!(normalise_symbol("brk.b"), Ok("BRK.B".to_string()));
        assert!(normalise_symbol("").is_err());
        assert!(normalise_symbol("way too long a symbol").is_err());
        assert!(normalise_symbol("AA PL").is_err());
    }

    /// Buy 10 @ $150.00, then 5 more @ $180.00 → weighted average $160.00.
    /// (10×150 + 5×180) / 15 = 160, verified by hand before writing this code.
    #[test]
    fn test_weighted_avg_cost_matches_hand_calculation() {
        let ten_at_150 = 10 * 10i64.pow(QUANTITY_EXPONENT);
        let five_at_180 = 5 * 10i64.pow(QUANTITY_EXPONENT);

        let avg = weighted_avg_cost_minor(ten_at_150, 15000, five_at_180, 18000);
        assert_eq!(avg, 16000, "expected $160.00 in cents");
    }

    /// Opening a position is the degenerate case: existing quantity is zero, so
    /// the average is simply the price paid.
    #[test]
    fn test_weighted_avg_cost_for_a_new_position() {
        let qty = 10 * 10i64.pow(QUANTITY_EXPONENT);
        assert_eq!(weighted_avg_cost_minor(0, 0, qty, 15000), 15000);
    }

    /// A third buy must weight against the *already-blended* average, not the
    /// original purchase price.
    #[test]
    fn test_weighted_avg_cost_across_three_buys() {
        let one = 10i64.pow(QUANTITY_EXPONENT);
        let avg_after_two = weighted_avg_cost_minor(0, 0, one, 10000);
        let avg_after_two = weighted_avg_cost_minor(one, avg_after_two, one, 20000);
        assert_eq!(avg_after_two, 15000);

        let avg_after_three = weighted_avg_cost_minor(2 * one, avg_after_two, one, 30000);
        // (2×150 + 1×300) / 3 = 200
        assert_eq!(avg_after_three, 20000);
    }

    #[test]
    fn test_realized_pl_can_be_a_gain_or_a_loss() {
        let qty = 5 * 10i64.pow(QUANTITY_EXPONENT);
        // Bought at $150, sold at $180: gain of $30 × 5 = $150.
        assert_eq!(realized_pl_minor(qty, 18000, 15000), 15000);
        // Bought at $150, sold at $120: loss of $30 × 5 = $150.
        assert_eq!(realized_pl_minor(qty, 12000, 15000), -15000);
        // Sold at cost: no gain, no loss.
        assert_eq!(realized_pl_minor(qty, 15000, 15000), 0);
    }

    #[test]
    fn test_holding_cost_basis_and_market_value() {
        let qty = 10 * 10i64.pow(QUANTITY_EXPONENT);
        let h = holding(qty, 15000, Some(18000));

        assert_eq!(h.cost_basis_minor(), 150000, "$1500.00 cost basis");
        assert_eq!(h.market_value_minor(), Some(180000), "$1800.00 market value");
        assert_eq!(h.unrealized_pl_minor(), Some(30000), "$300.00 unrealized gain");
    }

    /// An unpriced holding has no market value and no unrealized P&L — `None`,
    /// not zero, so a caller cannot mistake "unknown" for "flat".
    #[test]
    fn test_holding_without_a_price_has_no_market_value() {
        let h = holding(10 * 10i64.pow(QUANTITY_EXPONENT), 15000, None);
        assert_eq!(h.market_value_minor(), None);
        assert_eq!(h.unrealized_pl_minor(), None);
    }

    /// A position at a loss must report a negative unrealized P&L, not clamp to
    /// zero — that is exactly the number an investor needs to see.
    #[test]
    fn test_holding_at_a_loss_reports_negative_unrealized_pl() {
        let h = holding(10 * 10i64.pow(QUANTITY_EXPONENT), 15000, Some(12000));
        assert_eq!(h.unrealized_pl_minor(), Some(-30000));
    }
}
