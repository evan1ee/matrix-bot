//! Pure foreign exchange (FX) rate calculation and XML parsing.
//!
//! Provides cross-currency conversion math without modifying stored database minor units.
//! Network fetching of live rates is handled in the infrastructure layer.

use std::collections::HashMap;

use super::money::exponent_for;

/// Baseline fallback rates (against 1.0 EUR) used only if external feeds are unreachable on cold boot.
pub fn baseline_rates() -> HashMap<String, f64> {
    let mut m = HashMap::new();
    m.insert("EUR".to_string(), 1.0);
    m.insert("USD".to_string(), 1.085);
    m.insert("GBP".to_string(), 0.855);
    m.insert("CNY".to_string(), 7.85);
    m.insert("JPY".to_string(), 165.0);
    m.insert("CHF".to_string(), 0.965);
    m.insert("CAD".to_string(), 1.485);
    m.insert("AUD".to_string(), 1.65);
    m.insert("HKD".to_string(), 8.48);
    m.insert("SGD".to_string(), 1.46);
    m
}

/// Parses `<Cube currency='USD' rate='1.0850'/>` elements from ECB XML.
pub fn parse_ecb_xml(xml: &str) -> HashMap<String, f64> {
    let mut rates = HashMap::new();
    rates.insert("EUR".to_string(), 1.0);

    for line in xml.lines() {
        if !line.contains("currency") || !line.contains("rate") {
            continue;
        }
        let curr = extract_attribute(line, "currency");
        let rate_str = extract_attribute(line, "rate");
        if let (Some(c), Some(r_str)) = (curr, rate_str) {
            if let Ok(val) = r_str.parse::<f64>() {
                if val > 0.0 {
                    rates.insert(c.to_ascii_uppercase(), val);
                }
            }
        }
    }
    rates
}

fn extract_attribute<'a>(line: &'a str, attr: &str) -> Option<&'a str> {
    let target = format!("{attr}=");
    let start_idx = line.find(&target)? + target.len();
    let rest = &line[start_idx..];
    let quote_char = rest.chars().next()?;
    if quote_char != '\'' && quote_char != '"' {
        return None;
    }
    let after_quote = &rest[1..];
    let end_idx = after_quote.find(quote_char)?;
    Some(&after_quote[..end_idx])
}

/// Looks up the cross rate between any two currencies from an EUR-base rate map.
/// Returns `None` if either currency is unknown or has a non-positive rate.
pub fn try_get_rate_between(from: &str, to: &str, rates: &HashMap<String, f64>) -> Option<f64> {
    let from_u = from.trim().to_ascii_uppercase();
    let to_u = to.trim().to_ascii_uppercase();

    if from_u == to_u {
        return Some(1.0);
    }

    let from_eur = if from_u == "EUR" {
        Some(1.0)
    } else {
        rates
            .get(&from_u)
            .copied()
            .filter(|&r| r.is_finite() && r > 0.0)
    }?;
    let to_eur = if to_u == "EUR" {
        Some(1.0)
    } else {
        rates
            .get(&to_u)
            .copied()
            .filter(|&r| r.is_finite() && r > 0.0)
    }?;

    Some(to_eur / from_eur)
}

/// Calculates the cross rate between any two currencies from an EUR-base rate map.
/// Returns `0.0` when either currency has no known FX rate so unknown currencies
/// are never silently valued at `1.0 EUR`.
pub fn get_rate_between(from: &str, to: &str, rates: &HashMap<String, f64>) -> f64 {
    try_get_rate_between(from, to, rates).unwrap_or(0.0)
}

/// Converts an amount in minor units from one currency to another using the provided rates.
/// Correctly accounts for differing currency exponents (e.g. USD 2 decimals -> JPY 0 decimals).
/// Returns `0` when no FX rate is known for the currency pair.
pub fn convert_minor_units(
    amount_minor: i64,
    from_currency: &str,
    to_currency: &str,
    rates: &HashMap<String, f64>,
) -> i64 {
    let from_u = from_currency.trim().to_ascii_uppercase();
    let to_u = to_currency.trim().to_ascii_uppercase();

    if from_u == to_u {
        return amount_minor;
    }

    let Some(rate) = try_get_rate_between(&from_u, &to_u, rates) else {
        return 0;
    };
    let from_exp = exponent_for(&from_u);
    let to_exp = exponent_for(&to_u);

    // Convert from source minor units to major units (floating point)
    let major_amount = (amount_minor as f64) / 10f64.powi(from_exp as i32);
    // Convert to target major units
    let converted_major = major_amount * rate;
    // Convert to target minor units, rounded to nearest integer
    (converted_major * 10f64.powi(to_exp as i32)).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ecb_xml() {
        let sample = r#"
            <Cube>
                <Cube time='2026-03-27'>
                    <Cube currency='USD' rate='1.0850'/>
                    <Cube currency='JPY' rate='165.25'/>
                    <Cube currency='GBP' rate='0.8550'/>
                </Cube>
            </Cube>
        "#;
        let rates = parse_ecb_xml(sample);
        assert_eq!(rates.get("EUR"), Some(&1.0));
        assert_eq!(rates.get("USD"), Some(&1.0850));
        assert_eq!(rates.get("JPY"), Some(&165.25));
        assert_eq!(rates.get("GBP"), Some(&0.8550));
    }

    #[test]
    fn test_cross_rates_and_conversion() {
        let mut rates = HashMap::new();
        rates.insert("EUR".to_string(), 1.0);
        rates.insert("USD".to_string(), 1.10);
        rates.insert("GBP".to_string(), 0.88);
        rates.insert("JPY".to_string(), 165.0);

        // EUR to USD: 100 EUR = 110 USD -> 10000 cents EUR = 11000 cents USD
        assert_eq!(convert_minor_units(10000, "EUR", "USD", &rates), 11000);

        // USD to EUR: 110 USD = 100 EUR -> 11000 cents USD = 10000 cents EUR
        assert_eq!(convert_minor_units(11000, "USD", "EUR", &rates), 10000);

        // USD to JPY: 100 USD (10000 cents) = (100 / 1.10) * 165 JPY = 15000 JPY
        assert_eq!(convert_minor_units(10000, "USD", "JPY", &rates), 15000);
    }

    #[test]
    fn unknown_currency_does_not_default_to_one_eur() {
        let rates = baseline_rates();
        assert_eq!(try_get_rate_between("VND", "USD", &rates), None);
        assert_eq!(get_rate_between("VND", "USD", &rates), 0.0);
        assert_eq!(convert_minor_units(25_000_000, "VND", "USD", &rates), 0);
    }
}
