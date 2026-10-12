use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

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
    if !symbol
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holding {
    pub id: i64,
    pub owner_id: AccountId,
    pub symbol: String,
    pub quantity_minor: i64,
    pub avg_cost_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub current_price_minor: Option<i64>,
    pub price_updated_at: Option<DateTime<Utc>>,
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
        self.current_price_minor
            .map(|price| scale_product(self.quantity_minor, price))
    }

    /// `market value - cost basis`, or `None` while no price has been recorded.
    /// May legitimately be negative.
    pub fn unrealized_pl_minor(&self) -> Option<i64> {
        self.market_value_minor()
            .map(|value| value - self.cost_basis_minor())
    }
}

/// `quantity_minor * price_minor / 10^QUANTITY_EXPONENT`, in minor currency
/// units. Quantity's scale cancels against the divisor, leaving the result at
/// `currency_exponent` scale — the same trick [`weighted_avg_cost_minor`] uses.
/// `i128` intermediates: a large crypto quantity times a large price can
/// approach `i64`'s ceiling.
pub(crate) fn scale_product(quantity_minor: i64, price_minor: i64) -> i64 {
    if quantity_minor <= 0 || price_minor <= 0 {
        return 0;
    }
    let divisor = 10i128.pow(QUANTITY_EXPONENT);
    let product = quantity_minor as i128 * price_minor as i128;
    let rounded = ((product + divisor / 2) / divisor) as i64;
    rounded.max(1)
}

/// Cash cost (or proceeds) of `quantity_minor` units at `price_minor` each.
pub fn cost_of(quantity_minor: i64, price_minor: i64) -> i64 {
    scale_product(quantity_minor, price_minor)
}

/// The new weighted-average cost after adding `new_qty_minor` at `new_price_minor`
/// to an existing `existing_qty_minor` held at `existing_avg_cost_minor`.
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

/// Totals for one currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioCurrencySummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub cost_basis_minor: i64,
    pub market_value_minor: i64,
    pub unrealized_pl_minor: i64,
    pub priced_holdings: i64,
    pub unpriced_holdings: i64,
    pub cash_minor: i64,
}

/// Cash available to spend on a buy, in one currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CashBalance {
    pub id: i64,
    pub owner_id: AccountId,
    pub currency: String,
    pub currency_exponent: i64,
    pub balance_minor: i64,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub enum BuyOutcome {
    Ok {
        holding: Box<Holding>,
        cash: CashBalance,
    },
    CurrencyMismatch {
        existing_currency: String,
    },
    InsufficientCash {
        available_minor: i64,
        needed_minor: i64,
    },
}

#[derive(Debug, Clone)]
pub enum SellOutcome {
    NotFound,
    ExceedsHolding {
        held_minor: i64,
    },
    Closed {
        realized_pl_minor: i64,
        cash: CashBalance,
    },
    Reduced {
        holding: Holding,
        realized_pl_minor: i64,
        cash: CashBalance,
    },
}

#[derive(Debug, Clone)]
pub enum WithdrawOutcome {
    Ok(CashBalance),
    InsufficientFunds { available_minor: i64 },
}

/// Builds aggregated currency summaries from holding positions and cash balances.
pub fn build_summaries(
    holdings: &[Holding],
    cash_balances: &[CashBalance],
) -> Vec<PortfolioCurrencySummary> {
    let mut summaries: Vec<PortfolioCurrencySummary> = Vec::new();

    let entry_for =
        |summaries: &mut Vec<PortfolioCurrencySummary>, currency: &str, exponent: i64| {
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

    for holding in holdings {
        entry_for(&mut summaries, &holding.currency, holding.currency_exponent);
        let entry = summaries
            .iter_mut()
            .find(|s| s.currency == holding.currency)
            .expect("just inserted");

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

    for cash in cash_balances {
        entry_for(&mut summaries, &cash.currency, cash.currency_exponent);
        let entry = summaries
            .iter_mut()
            .find(|s| s.currency == cash.currency)
            .expect("just inserted");
        entry.cash_minor = cash.balance_minor;
    }

    summaries.sort_by(|a, b| a.currency.cmp(&b.currency));
    summaries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_holding(
        symbol: &str,
        qty: i64,
        avg_cost: i64,
        price: Option<i64>,
        currency: &str,
    ) -> Holding {
        Holding {
            id: 1,
            owner_id: AccountId::from_seed("alice"),
            symbol: symbol.to_string(),
            quantity_minor: qty,
            avg_cost_minor: avg_cost,
            currency: currency.to_string(),
            currency_exponent: 2,
            current_price_minor: price,
            price_updated_at: None,
            note: None,
            created_at: None,
            updated_at: None,
        }
    }

    #[test]
    fn test_normalise_symbol() {
        assert_eq!(normalise_symbol("aapl").unwrap(), "AAPL");
        assert_eq!(normalise_symbol("brk.b").unwrap(), "BRK.B");
        assert_eq!(normalise_symbol("btc-usd").unwrap(), "BTC-USD");
        assert!(normalise_symbol("").is_err());
        assert!(normalise_symbol("TOOLONGSYMBOL123").is_err());
        assert!(normalise_symbol("BAD$YM").is_err());
    }

    #[test]
    fn test_cost_and_weighted_average() {
        let qty = 10 * 10i64.pow(QUANTITY_EXPONENT);
        let price = 15_000; // $150.00
        assert_eq!(cost_of(qty, price), 150_000);

        let new_avg = weighted_avg_cost_minor(qty, 15_000, qty, 20_000);
        assert_eq!(new_avg, 17_500);

        // L2: 0.001 share at $9.99 (999 cents) rounds to 1 cent instead of truncating to 0
        let frac_qty = 100_000; // 0.001 * 10^8
        assert_eq!(cost_of(frac_qty, 999), 1);
        // Any positive quantity * positive price costs at least 1 minor unit
        assert_eq!(cost_of(1, 1), 1);
    }

    #[test]
    fn test_realized_pl() {
        let qty = 5 * 10i64.pow(QUANTITY_EXPONENT);
        let pl = realized_pl_minor(qty, 18_000, 15_000);
        assert_eq!(pl, 15_000); // 5 * 30.00 = 150.00
    }

    #[test]
    fn test_summaries_combines_holdings_and_cash() {
        let qty = 10 * 10i64.pow(QUANTITY_EXPONENT);
        let h1 = sample_holding("AAPL", qty, 15_000, Some(17_000), "USD");
        let cash = CashBalance {
            id: 1,
            owner_id: AccountId::from_seed("alice"),
            currency: "USD".to_string(),
            currency_exponent: 2,
            balance_minor: 50_000,
            updated_at: None,
        };

        let summaries = build_summaries(&[h1], &[cash]);
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].currency, "USD");
        assert_eq!(summaries[0].cost_basis_minor, 150_000);
        assert_eq!(summaries[0].market_value_minor, 170_000);
        assert_eq!(summaries[0].unrealized_pl_minor, 20_000);
        assert_eq!(summaries[0].cash_minor, 50_000);
    }
}
