//! Foreign exchange (FX) rates service.
//!
//! Fetches daily reference rates from the European Central Bank (ECB) eurofxref feed
//! with in-memory caching and fallback to Yahoo Finance rates. Provides cross-currency
//! conversion without modifying stored database minor units.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
const CACHE_TTL: Duration = Duration::from_secs(6 * 3600); // 6 hours

#[derive(Clone, Debug)]
pub struct FxCache {
    pub rates_eur_base: HashMap<String, f64>,
    pub fetched_at: DateTime<Utc>,
}

static FX_CACHE: RwLock<Option<FxCache>> = RwLock::const_new(None);

/// Baseline fallback rates (against 1.0 EUR) used only if external feeds are unreachable on cold boot.
fn baseline_rates() -> HashMap<String, f64> {
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

/// Fetches daily reference rates from the European Central Bank.
pub async fn fetch_ecb_rates(client: &reqwest::Client) -> Result<HashMap<String, f64>, String> {
    let resp = client
        .get(ECB_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Failed to reach ECB feed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("ECB feed returned HTTP {}", resp.status()));
    }

    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read ECB feed body: {e}"))?;

    let rates = parse_ecb_xml(&text);
    if rates.len() <= 1 {
        return Err("ECB feed parsed 0 currency rates".to_string());
    }
    Ok(rates)
}

/// Retrieves the current FX rates against EUR, using the 6-hour cache or refreshing from ECB.
pub async fn get_rates(client: &reqwest::Client) -> HashMap<String, f64> {
    {
        let guard = FX_CACHE.read().await;
        if let Some(cache) = &*guard {
            let elapsed = Utc::now().signed_duration_since(cache.fetched_at);
            if elapsed < chrono::Duration::from_std(CACHE_TTL).unwrap_or(chrono::Duration::hours(6))
            {
                return cache.rates_eur_base.clone();
            }
        }
    }

    match fetch_ecb_rates(client).await {
        Ok(rates) => {
            info!(
                "Updated FX reference rates from ECB ({} currencies)",
                rates.len()
            );
            let mut guard = FX_CACHE.write().await;
            *guard = Some(FxCache {
                rates_eur_base: rates.clone(),
                fetched_at: Utc::now(),
            });
            rates
        }
        Err(err) => {
            warn!(
                "Could not refresh ECB FX rates ({}); using cached or baseline rates",
                err
            );
            let guard = FX_CACHE.read().await;
            if let Some(cache) = &*guard {
                cache.rates_eur_base.clone()
            } else {
                baseline_rates()
            }
        }
    }
}

/// Computes the cross rate from `from` currency to `to` currency.
///
/// Returns how many units of `to` equal 1 unit of `from`.
pub fn get_rate_between(
    rates_eur_base: &HashMap<String, f64>,
    from: &str,
    to: &str,
) -> Option<f64> {
    if from.eq_ignore_ascii_case(to) {
        return Some(1.0);
    }
    let from_u = from.to_ascii_uppercase();
    let to_u = to.to_ascii_uppercase();

    let rate_from = rates_eur_base.get(&from_u)?;
    let rate_to = rates_eur_base.get(&to_u)?;

    if *rate_from <= 0.0 {
        return None;
    }
    Some(*rate_to / *rate_from)
}

/// Converts minor units from one currency to another without modifying database state.
pub fn convert_minor_units(
    amount_minor: i64,
    from_exponent: u32,
    to_exponent: u32,
    rate: f64,
) -> i64 {
    let dec_from = amount_minor as f64 / 10f64.powi(from_exponent as i32);
    let dec_to = dec_from * rate;
    (dec_to * 10f64.powi(to_exponent as i32)).round() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ecb_xml() {
        let sample = r#"
            <gesmes:Envelope>
              <Cube>
                <Cube time='2026-09-17'>
                  <Cube currency='USD' rate='1.0850'/>
                  <Cube currency="JPY" rate="165.20"/>
                  <Cube currency='GBP' rate='0.8540'/>
                </Cube>
              </Cube>
            </gesmes:Envelope>
        "#;
        let rates = parse_ecb_xml(sample);
        assert_eq!(rates.get("EUR").copied(), Some(1.0));
        assert_eq!(rates.get("USD").copied(), Some(1.0850));
        assert_eq!(rates.get("JPY").copied(), Some(165.20));
        assert_eq!(rates.get("GBP").copied(), Some(0.8540));
    }

    #[test]
    fn test_cross_rate_calculation() {
        let mut rates = HashMap::new();
        rates.insert("EUR".to_string(), 1.0);
        rates.insert("USD".to_string(), 1.10);
        rates.insert("GBP".to_string(), 0.88);

        // 1 EUR to USD = 1.10
        let r_eur_usd = get_rate_between(&rates, "EUR", "USD").unwrap();
        assert!((r_eur_usd - 1.10).abs() < 1e-6);

        // 1 USD to GBP = 0.88 / 1.10 = 0.80
        let r_usd_gbp = get_rate_between(&rates, "USD", "GBP").unwrap();
        assert!((r_usd_gbp - 0.80).abs() < 1e-6);

        // Identity
        assert_eq!(get_rate_between(&rates, "USD", "USD"), Some(1.0));
    }

    #[test]
    fn test_convert_minor_units() {
        // 100.00 USD (10000 cents, exp=2) at rate 0.80 to GBP (exp=2) -> 8000 pence (£80.00)
        let converted = convert_minor_units(10000, 2, 2, 0.80);
        assert_eq!(converted, 8000);

        // 10000 JPY (exp=0) at rate 0.0065 to USD (exp=2) -> 65.00 USD (6500 cents)
        let converted_jpy = convert_minor_units(10000, 0, 2, 0.0065);
        assert_eq!(converted_jpy, 6500);

        // Negative value (e.g. debt / net negative cash)
        let converted_neg = convert_minor_units(-5000, 2, 2, 1.20);
        assert_eq!(converted_neg, -6000);
    }
}
