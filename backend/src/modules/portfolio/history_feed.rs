use chrono::{DateTime, Utc};
use reqwest::header::{HeaderMap, HeaderValue, COOKIE, SET_COOKIE, USER_AGENT};
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::modules::portfolio::price_feed::is_crypto_symbol;

const HTTP_TIMEOUT_SECS: u64 = 10;
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

static HISTORY_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static YAHOO_SESSION: RwLock<Option<(String, String)>> = RwLock::const_new(None);

fn get_client() -> &'static reqwest::Client {
    HISTORY_CLIENT.get_or_init(|| {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .default_headers(headers)
            .build()
            .expect("Failed to build HTTP client for portfolio history feed")
    })
}

// ---------------------------------------------------------------------------
// Coinbase Historical Candles Parser (Crypto)
// ---------------------------------------------------------------------------

pub async fn fetch_coinbase_candles(symbol: &str) -> Result<Vec<(String, f64)>, String> {
    let clean = symbol.trim().to_uppercase();
    let base_sym = clean.split('-').next().unwrap_or(&clean);
    let client = get_client();

    let url = format!(
        "https://api.exchange.coinbase.com/products/{}-USD/candles?granularity=86400",
        base_sym
    );

    let res = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !res.status().is_success() {
        return Err(format!(
            "Coinbase candles failed for {} with status {}",
            base_sym,
            res.status()
        ));
    }

    let text = res.text().await.map_err(|e| e.to_string())?;
    parse_coinbase_candles_json(&text)
}

pub fn parse_coinbase_candles_json(body: &str) -> Result<Vec<(String, f64)>, String> {
    // Coinbase returns: [[time, low, high, open, close, volume], ...]
    let raw: Vec<Vec<f64>> = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Coinbase candles JSON: {e}"))?;

    let mut points: Vec<(String, f64)> = Vec::with_capacity(raw.len());

    for candle in raw {
        if candle.len() >= 5 {
            let ts = candle[0] as i64;
            let close = candle[4];
            if close > 0.0 {
                let dt = DateTime::from_timestamp(ts, 0).unwrap_or_default();
                let date_str = dt.format("%Y-%m-%d").to_string();
                points.push((date_str, close));
            }
        }
    }

    // Sort ascending by date
    points.sort_by(|a, b| a.0.cmp(&b.0));
    // Deduplicate any repeated dates keeping latest
    points.dedup_by(|a, b| a.0 == b.0);

    Ok(points)
}

// ---------------------------------------------------------------------------
// Yahoo Finance Chart History Parser (Equities, ETFs, Indices, Crypto fallback)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct YahooChartResponse {
    chart: YahooChartData,
}

#[derive(Deserialize)]
struct YahooChartData {
    result: Option<Vec<YahooChartResult>>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct YahooChartResult {
    timestamp: Option<Vec<i64>>,
    indicators: YahooChartIndicators,
}

#[derive(Deserialize)]
struct YahooChartIndicators {
    quote: Option<Vec<YahooChartQuote>>,
}

#[derive(Deserialize)]
struct YahooChartQuote {
    close: Option<Vec<Option<f64>>>,
}

/// Retrieves or refreshes a Yahoo Finance session (cookie header and crumb token).
pub async fn get_yahoo_session() -> Result<(String, String), String> {
    {
        let guard = YAHOO_SESSION.read().await;
        if let Some(ref session) = *guard {
            return Ok(session.clone());
        }
    }

    let client = get_client();

    // 1. Fetch cookie from fc.yahoo.com
    let res = client
        .get("https://fc.yahoo.com")
        .send()
        .await
        .map_err(|e| format!("Failed to connect to fc.yahoo.com: {e}"))?;

    let cookies: Vec<String> = res
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|s| s.split(';').next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let cookie_header = cookies.join("; ");

    // 2. Fetch crumb token using cookie
    let crumb_res = client
        .get("https://query2.finance.yahoo.com/v1/test/getcrumb")
        .header(COOKIE, &cookie_header)
        .send()
        .await
        .map_err(|e| format!("Failed to getcrumb from Yahoo: {e}"))?;

    if !crumb_res.status().is_success() {
        return Err(format!(
            "Yahoo getcrumb returned status {}",
            crumb_res.status()
        ));
    }

    let crumb = crumb_res
        .text()
        .await
        .map_err(|e| e.to_string())?
        .trim()
        .to_string();
    if crumb.is_empty() {
        return Err("Yahoo returned empty crumb".to_string());
    }

    let session = (cookie_header, crumb);
    let mut guard = YAHOO_SESSION.write().await;
    *guard = Some(session.clone());

    Ok(session)
}

pub fn parse_yahoo_chart_history_json(body: &str) -> Result<Vec<(String, f64)>, String> {
    let resp: YahooChartResponse = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Yahoo chart response: {e}"))?;

    if let Some(err) = resp.chart.error {
        if !err.is_null() {
            return Err(format!("Yahoo Finance chart returned error: {:?}", err));
        }
    }

    let first = resp
        .chart
        .result
        .and_then(|mut r| {
            if r.is_empty() {
                None
            } else {
                Some(r.remove(0))
            }
        })
        .ok_or_else(|| "Empty result list from Yahoo Finance chart".to_string())?;

    let timestamps = first.timestamp.unwrap_or_default();
    let quotes = first.indicators.quote.unwrap_or_default();
    let closes = quotes
        .into_iter()
        .next()
        .and_then(|q| q.close)
        .unwrap_or_default();

    let mut points: Vec<(String, f64)> = Vec::new();
    let mut last_valid_price: Option<f64> = None;

    for (ts, maybe_close) in timestamps.into_iter().zip(closes.into_iter()) {
        let price = match maybe_close {
            Some(p) if p > 0.0 => {
                last_valid_price = Some(p);
                p
            }
            _ => match last_valid_price {
                Some(prev) => prev,
                None => continue,
            },
        };

        let dt = DateTime::from_timestamp(ts, 0).unwrap_or_default();
        let date_str = dt.format("%Y-%m-%d").to_string();
        points.push((date_str, price));
    }

    points.sort_by(|a, b| a.0.cmp(&b.0));
    points.dedup_by(|a, b| a.0 == b.0);

    Ok(points)
}

pub async fn fetch_yahoo_chart_history(
    symbol: &str,
    range: &str,
) -> Result<Vec<(String, f64)>, String> {
    let client = get_client();
    let (cookie, crumb) = get_yahoo_session().await?;

    let yf_sym = if is_crypto_symbol(symbol) && !symbol.contains('-') {
        format!("{}-USD", symbol.trim().to_uppercase())
    } else {
        symbol.trim().to_uppercase()
    };

    let url = format!(
        "https://query2.finance.yahoo.com/v8/finance/chart/{}?crumb={}&interval=1d&range={}",
        yf_sym, crumb, range
    );

    let res = client
        .get(&url)
        .header(COOKIE, &cookie)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !res.status().is_success() {
        return Err(format!(
            "Yahoo chart failed for {} with status {}",
            yf_sym,
            res.status()
        ));
    }

    let text = res.text().await.map_err(|e| e.to_string())?;
    parse_yahoo_chart_history_json(&text)
}

// ---------------------------------------------------------------------------
// Unified Historical Price Fetcher
// ---------------------------------------------------------------------------

/// Maps a timeframe string ("7d", "1m", "3m", "1y", "all") to Yahoo Finance range.
pub fn timeframe_to_yahoo_range(timeframe: &str) -> &'static str {
    match timeframe.to_lowercase().as_str() {
        "7d" => "7d",
        "1m" => "1mo",
        "3m" => "3mo",
        "1y" => "1y",
        _ => "5y",
    }
}

/// Fetches daily historical closing prices for a symbol matching the given timeframe.
pub async fn fetch_historical_prices(
    symbol: &str,
    timeframe: &str,
) -> Result<Vec<(String, f64)>, String> {
    let clean = symbol.trim().to_uppercase();

    // Try Coinbase for crypto symbols first
    if is_crypto_symbol(&clean) {
        if let Ok(points) = fetch_coinbase_candles(&clean).await {
            if !points.is_empty() {
                return Ok(filter_by_timeframe(points, timeframe));
            }
        }
    }

    // Fallback or equity lookup via Yahoo Finance
    let range = timeframe_to_yahoo_range(timeframe);
    match fetch_yahoo_chart_history(&clean, range).await {
        Ok(points) if !points.is_empty() => Ok(filter_by_timeframe(points, timeframe)),
        Ok(_) => Err(format!("No historical data returned for {clean}")),
        Err(err) => Err(format!("Historical lookup failed for {clean}: {err}")),
    }
}

fn filter_by_timeframe(points: Vec<(String, f64)>, timeframe: &str) -> Vec<(String, f64)> {
    let days = match timeframe.to_lowercase().as_str() {
        "7d" => 7,
        "1m" => 30,
        "3m" => 90,
        "1y" => 365,
        _ => 3650,
    };

    let cutoff = (Utc::now() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();

    points
        .into_iter()
        .filter(|(date, _)| date >= &cutoff)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_coinbase_candles_json() {
        let json = r#"[
            [1788652800, 79829.84, 80060.99, 79833.86, 80028.12, 294.13],
            [1788566400, 79458.74, 80200.67, 79675.12, 79831.57, 2147.97]
        ]"#;

        let points = parse_coinbase_candles_json(json).unwrap();
        assert_eq!(points.len(), 2);
        // Sorted ascending
        assert_eq!(points[0].1, 79831.57);
        assert_eq!(points[1].1, 80028.12);
    }

    #[test]
    fn test_parse_yahoo_chart_history_json() {
        let json = r#"{
            "chart": {
                "result": [{
                    "timestamp": [1788134400, 1788220800, 1788307200],
                    "indicators": {
                        "quote": [{
                            "close": [320.5, null, 325.75]
                        }]
                    }
                }],
                "error": null
            }
        }"#;

        let points = parse_yahoo_chart_history_json(json).unwrap();
        assert_eq!(points.len(), 3);
        assert_eq!(points[0].1, 320.5);
        // Forward filled null price
        assert_eq!(points[1].1, 320.5);
        assert_eq!(points[2].1, 325.75);
    }

    #[test]
    fn test_timeframe_to_yahoo_range() {
        assert_eq!(timeframe_to_yahoo_range("7d"), "7d");
        assert_eq!(timeframe_to_yahoo_range("1m"), "1mo");
        assert_eq!(timeframe_to_yahoo_range("3m"), "3mo");
        assert_eq!(timeframe_to_yahoo_range("1y"), "1y");
        assert_eq!(timeframe_to_yahoo_range("all"), "5y");
    }
}
