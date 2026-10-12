//! Money handling for the finance domain.
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

impl std::error::Error for MoneyError {}

/// Widely used codes, so a lowercase `eur` is understood without treating every
/// three-letter word as a currency.
const COMMON_CODES: &[&str] = &[
    "AED", "ARS", "AUD", "BGN", "BRL", "CAD", "CHF", "CLP", "CNY", "COP", "CZK", "DKK", "EGP",
    "EUR", "GBP", "HKD", "HUF", "IDR", "ILS", "INR", "ISK", "JPY", "KES", "KRW", "MXN", "MYR",
    "NGN", "NOK", "NZD", "PHP", "PLN", "RON", "RUB", "SAR", "SEK", "SGD", "THB", "TRY", "TWD",
    "UAH", "USD", "VND", "ZAR",
];

/// Whether a bare token in currency position should be read as a currency.
pub fn looks_like_currency(token: &str) -> bool {
    if token.len() != 3 || !token.chars().all(|c| c.is_ascii_alphabetic()) {
        return false;
    }
    let all_caps = token.chars().all(|c| c.is_ascii_uppercase());
    all_caps || COMMON_CODES.contains(&token.to_uppercase().as_str())
}

/// Normalises a currency code, rejecting anything that is not three letters.
pub fn normalise_currency(raw: &str) -> Result<String, MoneyError> {
    crate::domain::settings::parse_currency(raw).map_err(|_| MoneyError::InvalidCurrency)
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

/// Formats minor units for display.
pub fn format_amount(minor: i64, exponent: u32) -> String {
    let sign = if minor < 0 { "-" } else { "" };
    let abs = minor.unsigned_abs();

    if exponent == 0 {
        return format!("{sign}{abs}");
    }

    let divisor = 10u64.pow(exponent);
    let major = abs / divisor;
    let fractional = abs % divisor;
    format!(
        "{sign}{major}.{fractional:0>width$}",
        width = exponent as usize
    )
}

/// Converts a finite positive `f64` into a plain decimal string without forcing
/// 2 decimal places (`{:.2}`), so zero-decimal currencies (`JPY`), three-decimal
/// currencies (`KWD`), and fractional prices are preserved for [`parse_amount`].
pub fn format_decimal_f64(num: f64) -> Result<String, MoneyError> {
    if !num.is_finite() {
        return Err(MoneyError::InvalidAmount);
    }
    if num <= 0.0 {
        return Err(MoneyError::NotPositive);
    }
    let s = num.to_string();
    if s.contains(['e', 'E']) {
        let fixed = format!("{num:.8}");
        let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
        if trimmed.is_empty() || trimmed == "0" {
            return Err(MoneyError::NotPositive);
        }
        return Ok(trimmed.to_string());
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponent_for() {
        assert_eq!(exponent_for("EUR"), 2);
        assert_eq!(exponent_for("USD"), 2);
        assert_eq!(exponent_for("JPY"), 0);
        assert_eq!(exponent_for("KWD"), 3);
        assert_eq!(exponent_for("jpy"), 0);
    }

    #[test]
    fn test_parse_amount_two_decimals() {
        assert_eq!(parse_amount("15.50", 2).unwrap(), 1550);
        assert_eq!(parse_amount("15", 2).unwrap(), 1500);
        assert_eq!(parse_amount("0.5", 2).unwrap(), 50);
        assert_eq!(parse_amount(".50", 2).unwrap(), 50);
        assert_eq!(parse_amount("1,000.50", 2).unwrap(), 100050);
        assert_eq!(parse_amount("$15.50", 2).unwrap(), 1550);
    }

    #[test]
    fn test_parse_amount_zero_decimals() {
        assert_eq!(parse_amount("1500", 0).unwrap(), 1500);
        assert!(matches!(
            parse_amount("15.5", 0),
            Err(MoneyError::TooPrecise { max_decimals: 0 })
        ));
    }

    #[test]
    fn test_parse_amount_three_decimals() {
        assert_eq!(parse_amount("1.234", 3).unwrap(), 1234);
        assert_eq!(parse_amount("1.2", 3).unwrap(), 1200);
        assert!(matches!(
            parse_amount("1.2345", 3),
            Err(MoneyError::TooPrecise { max_decimals: 3 })
        ));
    }

    #[test]
    fn test_parse_amount_invalid() {
        assert!(matches!(parse_amount("0", 2), Err(MoneyError::NotPositive)));
        assert!(matches!(
            parse_amount("-5", 2),
            Err(MoneyError::NotPositive)
        ));
        assert!(matches!(
            parse_amount("abc", 2),
            Err(MoneyError::InvalidAmount)
        ));
        assert!(matches!(
            parse_amount("", 2),
            Err(MoneyError::InvalidAmount)
        ));
    }

    #[test]
    fn test_format_amount() {
        assert_eq!(format_amount(1550, 2), "15.50");
        assert_eq!(format_amount(50, 2), "0.50");
        assert_eq!(format_amount(1500, 0), "1500");
        assert_eq!(format_amount(1234, 3), "1.234");
        assert_eq!(format_amount(-1550, 2), "-15.50");
    }

    #[test]
    fn test_looks_like_currency() {
        assert!(looks_like_currency("EUR"));
        assert!(looks_like_currency("eur"));
        assert!(looks_like_currency("USD"));
        assert!(looks_like_currency("jpy"));
        assert!(!looks_like_currency("tea"));
        assert!(!looks_like_currency("lunch"));
        assert!(!looks_like_currency("EU"));
        assert!(!looks_like_currency("EURO"));
    }

    #[test]
    fn test_format_decimal_f64() {
        assert_eq!(format_decimal_f64(1500.0).unwrap(), "1500");
        assert_eq!(
            parse_amount(&format_decimal_f64(1500.0).unwrap(), 0).unwrap(),
            1500
        );
        assert_eq!(format_decimal_f64(1.234).unwrap(), "1.234");
        assert_eq!(
            parse_amount(&format_decimal_f64(1.234).unwrap(), 3).unwrap(),
            1234
        );
        assert_eq!(format_decimal_f64(0.004).unwrap(), "0.004");
        assert!(format_decimal_f64(f64::INFINITY).is_err());
        assert!(format_decimal_f64(0.0).is_err());
    }
}
