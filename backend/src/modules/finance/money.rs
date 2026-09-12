//! Money handling for the finance module.
//!
//! Amounts are integer **minor units** — cents, not euros — and are never
//! carried through a float at any point. `"15.50"` becomes `1550` by string
//! manipulation, so no value can be lost to binary rounding on the way in.
//!
//! Minor units are not universally two decimal places: JPY has none, KWD has
//! three. Each transaction stores the exponent it was written with, so a change
//! to the table below can never silently rewrite the displayed value of a row
//! that already exists.

use std::fmt;

/// Currencies whose minor unit is not 1/100. Everything absent from this list
/// uses the near-universal exponent of 2.
const ZERO_DECIMAL: &[&str] = &[
    "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG", "RWF", "UGX", "UYI", "VND",
    "VUV", "XAF", "XOF", "XPF",
];
const THREE_DECIMAL: &[&str] = &["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MoneyError {
    /// Not a currency code we can make sense of.
    InvalidCurrency,
    /// Not a number, or a number in a shape we refuse to guess at.
    InvalidAmount,
    /// More decimal places than the currency has.
    TooPrecise { max_decimals: u32 },
    /// Zero or negative. Direction carries the sign, so the amount must not.
    NotPositive,
    /// Large enough to overflow the column.
    OutOfRange,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MoneyError::InvalidCurrency => {
                write!(f, "Currency must be a 3-letter code such as EUR or JPY")
            }
            MoneyError::InvalidAmount => write!(f, "Amount must be a number such as 15.50"),
            MoneyError::TooPrecise { max_decimals } => {
                write!(f, "That currency has {} decimal place(s)", max_decimals)
            }
            MoneyError::NotPositive => {
                write!(f, "Amount must be greater than zero")
            }
            MoneyError::OutOfRange => write!(f, "Amount is too large"),
        }
    }
}

/// Widely used codes, so a lowercase `eur` is understood without treating every
/// three-letter word as a currency.
const COMMON_CODES: &[&str] = &[
    "AED", "ARS", "AUD", "BGN", "BRL", "CAD", "CHF", "CLP", "CNY", "COP", "CZK", "DKK", "EGP",
    "EUR", "GBP", "HKD", "HUF", "IDR", "ILS", "INR", "ISK", "JPY", "KES", "KRW", "MXN", "MYR",
    "NGN", "NOK", "NZD", "PHP", "PLN", "RON", "RUB", "SAR", "SEK", "SGD", "THB", "TRY", "TWD",
    "UAH", "USD", "VND", "ZAR",
];

/// Whether a bare token in currency position should be read as a currency.
///
/// Requires either all-caps (`EUR`) or membership of the common list (`eur`).
/// Without this, `!expense add 3.00 tea and cake` books a currency called TEA
/// and silently drops the word from the note.
pub fn looks_like_currency(token: &str) -> bool {
    if token.len() != 3 || !token.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let all_caps = token.chars().all(|c| c.is_ascii_uppercase());
    all_caps || COMMON_CODES.contains(&token.to_uppercase().as_str())
}

/// Normalises a currency code, rejecting anything that is not three letters.
pub fn normalise_currency(raw: &str) -> Result<String, MoneyError> {
    let code = raw.trim().to_uppercase();
    if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
        Ok(code)
    } else {
        Err(MoneyError::InvalidCurrency)
    }
}

/// Decimal places for a currency's minor unit.
pub fn exponent_for(currency: &str) -> u32 {
    let code = currency.to_uppercase();
    if ZERO_DECIMAL.contains(&code.as_str()) {
        0
    } else if THREE_DECIMAL.contains(&code.as_str()) {
        3
    } else {
        2
    }
}

/// Parses a human-typed amount into minor units.
///
/// Accepts a leading currency symbol, thousands separators, and a bare decimal
/// point (`.50`). Rejects a negative sign outright — `direction` is what makes a
/// transaction an expense, and accepting a sign here would allow rows whose sign
/// and direction disagree.
pub fn parse_amount(input: &str, exponent: u32) -> Result<i64, MoneyError> {
    let cleaned: String = input
        .trim()
        .trim_start_matches(['$', '€', '£', '¥', '₹', '₩'])
        .chars()
        .filter(|c| *c != ',' && *c != '_' && !c.is_whitespace())
        .collect();

    if cleaned.is_empty() {
        return Err(MoneyError::InvalidAmount);
    }
    if cleaned.starts_with('-') {
        return Err(MoneyError::NotPositive);
    }

    let (whole, fraction) = match cleaned.split_once('.') {
        Some((w, f)) => (w, f),
        None => (cleaned.as_str(), ""),
    };

    // `.5` is fine; `5.` is not, and neither is `1.2.3`.
    if fraction.contains('.') || (whole.is_empty() && fraction.is_empty()) {
        return Err(MoneyError::InvalidAmount);
    }
    if !whole.chars().all(|c| c.is_ascii_digit()) || !fraction.chars().all(|c| c.is_ascii_digit()) {
        return Err(MoneyError::InvalidAmount);
    }
    if fraction.len() as u32 > exponent {
        return Err(MoneyError::TooPrecise {
            max_decimals: exponent,
        });
    }

    let padded = format!("{:0<width$}", fraction, width = exponent as usize);
    let combined = format!("{}{}", if whole.is_empty() { "0" } else { whole }, padded);

    let minor: i64 = combined.parse().map_err(|_| MoneyError::OutOfRange)?;
    if minor <= 0 {
        return Err(MoneyError::NotPositive);
    }
    Ok(minor)
}

/// Renders minor units back as a decimal string. Never lossy.
pub fn format_amount(minor: i64, exponent: u32) -> String {
    let negative = minor < 0;
    let magnitude = minor.unsigned_abs();

    let text = if exponent == 0 {
        magnitude.to_string()
    } else {
        let divisor = 10u64.pow(exponent);
        let whole = magnitude / divisor;
        let frac = magnitude % divisor;
        format!("{}.{:0>width$}", whole, frac, width = exponent as usize)
    };

    if negative {
        format!("-{}", text)
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_common_shapes() {
        assert_eq!(parse_amount("15.50", 2), Ok(1550));
        assert_eq!(parse_amount("15.5", 2), Ok(1550));
        assert_eq!(parse_amount("15", 2), Ok(1500));
        assert_eq!(parse_amount(".50", 2), Ok(50));
        assert_eq!(parse_amount("$15.50", 2), Ok(1550));
        assert_eq!(parse_amount("1,000.50", 2), Ok(100050));
        assert_eq!(parse_amount(" 15.50 ", 2), Ok(1550));
    }

    /// The classic float trap: 0.1 + 0.2 must not drift, and a value that has
    /// no exact binary representation must survive exactly.
    #[test]
    fn test_no_float_drift() {
        assert_eq!(parse_amount("0.10", 2), Ok(10));
        assert_eq!(parse_amount("0.20", 2), Ok(20));
        assert_eq!(parse_amount("0.29", 2), Ok(29));
        assert_eq!(parse_amount("1234567.89", 2), Ok(123456789));
        // Round-trips exactly, which a float would not guarantee.
        assert_eq!(format_amount(parse_amount("0.29", 2).unwrap(), 2), "0.29");
        assert_eq!(
            format_amount(parse_amount("1234567.89", 2).unwrap(), 2),
            "1234567.89"
        );
    }

    #[test]
    fn test_respects_currency_exponent() {
        // JPY has no minor unit at all.
        assert_eq!(parse_amount("500", 0), Ok(500));
        assert_eq!(
            parse_amount("500.5", 0),
            Err(MoneyError::TooPrecise { max_decimals: 0 })
        );
        // KWD has three.
        assert_eq!(parse_amount("1.234", 3), Ok(1234));
        assert_eq!(
            parse_amount("1.2345", 3),
            Err(MoneyError::TooPrecise { max_decimals: 3 })
        );
    }

    #[test]
    fn test_rejects_nonsense() {
        assert_eq!(parse_amount("", 2), Err(MoneyError::InvalidAmount));
        assert_eq!(parse_amount("abc", 2), Err(MoneyError::InvalidAmount));
        assert_eq!(parse_amount("1.2.3", 2), Err(MoneyError::InvalidAmount));
        assert_eq!(
            parse_amount("15.501", 2),
            Err(MoneyError::TooPrecise { max_decimals: 2 })
        );
        assert_eq!(parse_amount("0", 2), Err(MoneyError::NotPositive));
        assert_eq!(parse_amount("0.00", 2), Err(MoneyError::NotPositive));
    }

    /// Direction is what makes a transaction an expense. A signed amount would
    /// allow a row whose sign and direction contradict each other.
    #[test]
    fn test_rejects_a_signed_amount() {
        assert_eq!(parse_amount("-15.50", 2), Err(MoneyError::NotPositive));
    }

    #[test]
    fn test_formats_back() {
        assert_eq!(format_amount(1550, 2), "15.50");
        assert_eq!(format_amount(50, 2), "0.50");
        assert_eq!(format_amount(5, 2), "0.05");
        assert_eq!(format_amount(100050, 2), "1000.50");
        assert_eq!(format_amount(500, 0), "500");
        assert_eq!(format_amount(1234, 3), "1.234");
        assert_eq!(format_amount(-1550, 2), "-15.50");
    }

    #[test]
    fn test_currency_normalisation_and_exponents() {
        assert_eq!(normalise_currency("eur"), Ok("EUR".to_string()));
        assert_eq!(normalise_currency(" usd "), Ok("USD".to_string()));
        assert_eq!(normalise_currency("EURO"), Err(MoneyError::InvalidCurrency));
        assert_eq!(normalise_currency("E1R"), Err(MoneyError::InvalidCurrency));

        assert_eq!(exponent_for("EUR"), 2);
        assert_eq!(exponent_for("jpy"), 0);
        assert_eq!(exponent_for("KWD"), 3);
        // Unknown codes fall back to the near-universal case rather than failing.
        assert_eq!(exponent_for("ZZZ"), 2);
    }
}
