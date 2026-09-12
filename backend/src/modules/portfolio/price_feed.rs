use reqwest::header::{HeaderMap, HeaderValue, USER_AGENT};
use serde::Deserialize;
use sqlx::SqlitePool;
use std::sync::OnceLock;
use std::time::Duration;
use tracing::{error, info, warn};

use crate::events::{EventBroadcaster, SystemEvent};
use crate::modules::portfolio::models::Holding;
use crate::modules::portfolio::repository::PortfolioRepository;

const HTTP_TIMEOUT_SECS: u64 = 8;
const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

static PRICE_FEED_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn get_client() -> &'static reqwest::Client {
    PRICE_FEED_CLIENT.get_or_init(|| {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(BROWSER_USER_AGENT));
        reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .default_headers(headers)
            .build()
            .expect("Failed to build HTTP client for portfolio price feed")
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct FetchedPrice {
    pub price: f64,
    pub currency: String,
    pub change_percent: Option<f64>,
}

#[derive(Debug, Default, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct RefreshSummary {
    pub updated: Vec<String>,
    pub failed: Vec<String>,
}

/// Known crypto symbols where Coinbase/CoinGecko spot lookup is primary.
const KNOWN_CRYPTO: &[&str] = &[
    "BTC", "ETH", "SOL", "DOGE", "ADA", "XRP", "DOT", "AVAX", "MATIC", "LINK", "LTC", "BCH", "XLM",
    "UNI", "ATOM", "XMR", "ETC", "NEAR", "ALGO", "FIL",
];

pub fn is_crypto_symbol(symbol: &str) -> bool {
    let clean = symbol.trim().to_uppercase();
    KNOWN_CRYPTO.contains(&clean.as_str()) || clean.ends_with("-USD") || clean.ends_with("-USDT")
}

// ---------------------------------------------------------------------------
// Yahoo Finance Parser (Equities, ETFs, Indices)
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
    meta: YahooChartMeta,
}

#[derive(Deserialize)]
struct YahooChartMeta {
    currency: Option<String>,
    #[serde(rename = "regularMarketPrice")]
    regular_market_price: Option<f64>,
    #[serde(rename = "regularMarketChangePercent")]
    regular_market_change_percent: Option<f64>,
}

pub fn parse_yahoo_chart_json(body: &str) -> Result<FetchedPrice, String> {
    let resp: YahooChartResponse = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Yahoo Finance response: {}", e))?;

    if let Some(err) = resp.chart.error {
        if !err.is_null() {
            return Err(format!("Yahoo Finance returned error: {:?}", err));
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
        .ok_or_else(|| "Empty result list from Yahoo Finance".to_string())?;

    let price = first
        .meta
        .regular_market_price
        .ok_or_else(|| "No regularMarketPrice in quote".to_string())?;
    let currency = first.meta.currency.unwrap_or_else(|| "USD".to_string());

    Ok(FetchedPrice {
        price,
        currency,
        change_percent: first.meta.regular_market_change_percent,
    })
}

// ---------------------------------------------------------------------------
// Coinbase Parser (Crypto Spot Price)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct CoinbaseSpotResponse {
    data: CoinbaseSpotData,
}

#[derive(Deserialize)]
struct CoinbaseSpotData {
    amount: String,
    currency: String,
}

pub fn parse_coinbase_spot_json(body: &str) -> Result<FetchedPrice, String> {
    let resp: CoinbaseSpotResponse = serde_json::from_str(body)
        .map_err(|e| format!("Failed to parse Coinbase response: {}", e))?;

    let price = resp
        .data
        .amount
        .parse::<f64>()
        .map_err(|e| format!("Invalid numeric price from Coinbase: {}", e))?;

    Ok(FetchedPrice {
        price,
        currency: resp.data.currency,
        change_percent: None,
    })
}

// ---------------------------------------------------------------------------
// Public Fetch Functions
// ---------------------------------------------------------------------------

/// Fetches the latest market price for a symbol using free public APIs.
pub async fn fetch_latest_price(symbol: &str) -> Result<FetchedPrice, String> {
    let clean_sym = symbol.trim().to_uppercase();
    let client = get_client();

    if is_crypto_symbol(&clean_sym) {
        let base_sym = clean_sym.split('-').next().unwrap_or(&clean_sym);
        let cb_url = format!("https://api.coinbase.com/v2/prices/{}-USD/spot", base_sym);

        if let Ok(res) = client.get(&cb_url).send().await {
            if res.status().is_success() {
                if let Ok(body) = res.text().await {
                    if let Ok(parsed) = parse_coinbase_spot_json(&body) {
                        return Ok(parsed);
                    }
                }
            }
        }

        // Fallback to Yahoo Finance for crypto (e.g. BTC-USD)
        let yf_sym = if clean_sym.contains('-') {
            clean_sym.clone()
        } else {
            format!("{}-USD", clean_sym)
        };
        let session = crate::modules::portfolio::history_feed::get_yahoo_session()
            .await
            .ok();
        let yf_url = if let Some((_, ref crumb)) = session {
            format!(
                "https://query2.finance.yahoo.com/v8/finance/chart/{}?crumb={}&interval=1d&range=1d",
                yf_sym, crumb
            )
        } else {
            format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1d&range=1d",
                yf_sym
            )
        };

        let mut req = client.get(&yf_url);
        if let Some((ref cookie, _)) = session {
            req = req.header(reqwest::header::COOKIE, cookie);
        }

        let res = req.send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!(
                "Price lookup failed for {} with status {}",
                yf_sym,
                res.status()
            ));
        }
        let body = res.text().await.map_err(|e| e.to_string())?;
        parse_yahoo_chart_json(&body)
    } else {
        // Stocks / ETFs / Indices
        let session = crate::modules::portfolio::history_feed::get_yahoo_session()
            .await
            .ok();
        let yf_url = if let Some((_, ref crumb)) = session {
            format!(
                "https://query2.finance.yahoo.com/v8/finance/chart/{}?crumb={}&interval=1d&range=1d",
                clean_sym, crumb
            )
        } else {
            format!(
                "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1d&range=1d",
                clean_sym
            )
        };

        let mut req = client.get(&yf_url);
        if let Some((ref cookie, _)) = session {
            req = req.header(reqwest::header::COOKIE, cookie);
        }

        let res = req.send().await.map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!(
                "Yahoo Finance returned status {} for {}",
                res.status(),
                clean_sym
            ));
        }
        let body = res.text().await.map_err(|e| e.to_string())?;
        parse_yahoo_chart_json(&body)
    }
}

/// Converts a floating-point market price to integer minor units matching the holding's currency exponent.
pub fn price_to_minor(price: f64, exponent: i64) -> i64 {
    let scale = 10f64.powi(exponent.clamp(0, 8) as i32);
    (price * scale).round() as i64
}

/// Refreshes the price for an individual holding and broadcasts an update event.
pub async fn refresh_holding_price(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    holding: &Holding,
) -> Result<Holding, String> {
    let fetched = fetch_latest_price(&holding.symbol).await?;

    let price_minor = price_to_minor(fetched.price, holding.currency_exponent);

    let updated =
        PortfolioRepository::set_price(pool, &holding.owner_id, &holding.symbol, price_minor)
            .await
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Holding for {} not found", holding.symbol))?;

    if let Some(b) = broadcaster {
        b.send(SystemEvent::PortfolioHoldingUpdated(updated.clone()));
    }

    info!(
        "Refreshed price for {} (owner {}): {:.2} {} ({} minor units)",
        holding.symbol, holding.owner_id, fetched.price, holding.currency, price_minor
    );

    Ok(updated)
}

/// Refreshes prices for all holdings owned by the given owner ID.
pub async fn refresh_all_for_owner(
    pool: &SqlitePool,
    broadcaster: Option<&EventBroadcaster>,
    owner_id: &str,
) -> RefreshSummary {
    let mut summary = RefreshSummary::default();

    let holdings = match PortfolioRepository::list(pool, owner_id).await {
        Ok(h) => h,
        Err(e) => {
            error!("Failed to list holdings for owner {}: {}", owner_id, e);
            return summary;
        }
    };

    for holding in holdings {
        match refresh_holding_price(pool, broadcaster, &holding).await {
            Ok(_) => summary.updated.push(holding.symbol),
            Err(err) => {
                warn!("Failed to refresh price for {}: {}", holding.symbol, err);
                summary.failed.push(holding.symbol);
            }
        }
    }

    summary
}

// ---------------------------------------------------------------------------
// Yahoo Finance Ticker Search & Autocomplete
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct TickerSearchResult {
    pub symbol: String,
    pub name: String,
    pub exchange: Option<String>,
    pub quote_type: Option<String>,
}

#[derive(Deserialize)]
struct YahooSearchResponse {
    quotes: Option<Vec<YahooSearchQuote>>,
}

#[derive(Deserialize)]
struct YahooSearchQuote {
    symbol: Option<String>,
    shortname: Option<String>,
    longname: Option<String>,
    #[serde(rename = "exchDisp")]
    exch_disp: Option<String>,
    #[serde(rename = "typeDisp")]
    type_disp: Option<String>,
    #[serde(rename = "quoteType")]
    quote_type: Option<String>,
}

pub fn parse_yahoo_search_json(json_str: &str) -> Result<Vec<TickerSearchResult>, String> {
    let parsed: YahooSearchResponse = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse Yahoo Search JSON: {e}"))?;

    let mut results = Vec::new();
    if let Some(quotes) = parsed.quotes {
        for q in quotes {
            if let Some(symbol) = q.symbol {
                if symbol.is_empty() {
                    continue;
                }
                let name = q.longname.or(q.shortname).unwrap_or_else(|| symbol.clone());
                results.push(TickerSearchResult {
                    symbol,
                    name,
                    exchange: q.exch_disp,
                    quote_type: q.type_disp.or(q.quote_type),
                });
            }
        }
    }

    Ok(results)
}

pub async fn search_symbols(query: &str) -> Result<Vec<TickerSearchResult>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }

    let client = get_client();
    let query_params = [("q", q), ("quotesCount", "8"), ("newsCount", "0")];

    let endpoints = [
        "https://query2.finance.yahoo.com/v1/finance/search",
        "https://query1.finance.yahoo.com/v1/finance/search",
    ];

    for endpoint in endpoints {
        if let Ok(resp) = client.get(endpoint).query(&query_params).send().await {
            if resp.status().is_success() {
                if let Ok(text) = resp.text().await {
                    if let Ok(results) = parse_yahoo_search_json(&text) {
                        if !results.is_empty() {
                            return Ok(results);
                        }
                    }
                }
            }
        }
    }

    // Static fallback for popular symbols if network is offline or blocked
    let popular = [
        ("AAPL", "Apple Inc.", "NASDAQ", "Equity"),
        ("MSFT", "Microsoft Corporation", "NASDAQ", "Equity"),
        ("NVDA", "NVIDIA Corporation", "NASDAQ", "Equity"),
        ("GOOGL", "Alphabet Inc.", "NASDAQ", "Equity"),
        ("AMZN", "Amazon.com, Inc.", "NASDAQ", "Equity"),
        ("TSLA", "Tesla, Inc.", "NASDAQ", "Equity"),
        ("META", "Meta Platforms, Inc.", "NASDAQ", "Equity"),
        ("BTC-USD", "Bitcoin USD", "CCC", "Cryptocurrency"),
        ("ETH-USD", "Ethereum USD", "CCC", "Cryptocurrency"),
        ("SOL-USD", "Solana USD", "CCC", "Cryptocurrency"),
        ("SPY", "SPDR S&P 500 ETF Trust", "NYSE", "ETF"),
        ("VOO", "Vanguard S&P 500 ETF", "NYSE", "ETF"),
        ("QQQ", "Invesco QQQ Trust", "NASDAQ", "ETF"),
    ];
    let q_upper = q.to_uppercase();
    let matches: Vec<TickerSearchResult> = popular
        .iter()
        .filter(|(sym, name, _, _)| {
            sym.contains(&q_upper) || name.to_uppercase().contains(&q_upper)
        })
        .map(|(sym, name, exch, qt)| TickerSearchResult {
            symbol: sym.to_string(),
            name: name.to_string(),
            exchange: Some(exch.to_string()),
            quote_type: Some(qt.to_string()),
        })
        .collect();

    Ok(matches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yahoo_chart_json() {
        let json = r#"{
            "chart": {
                "result": [{
                    "meta": {
                        "currency": "USD",
                        "symbol": "AAPL",
                        "regularMarketPrice": 324.96,
                        "regularMarketChangePercent": -0.052
                    }
                }],
                "error": null
            }
        }"#;

        let parsed = parse_yahoo_chart_json(json).unwrap();
        assert_eq!(parsed.price, 324.96);
        assert_eq!(parsed.currency, "USD");
        assert_eq!(parsed.change_percent, Some(-0.052));
    }

    #[test]
    fn test_parse_coinbase_spot_json() {
        let json = r#"{"data":{"amount":"77733.935","base":"BTC","currency":"USD"}}"#;
        let parsed = parse_coinbase_spot_json(json).unwrap();
        assert_eq!(parsed.price, 77733.935);
        assert_eq!(parsed.currency, "USD");
    }

    #[test]
    fn test_price_to_minor() {
        assert_eq!(price_to_minor(324.96, 2), 32496);
        assert_eq!(price_to_minor(77733.5, 2), 7773350);
        assert_eq!(price_to_minor(12.3456, 4), 123456);
    }

    #[test]
    fn test_is_crypto_symbol() {
        assert!(is_crypto_symbol("BTC"));
        assert!(is_crypto_symbol("ETH"));
        assert!(is_crypto_symbol("SOL"));
        assert!(is_crypto_symbol("BTC-USD"));
        assert!(!is_crypto_symbol("AAPL"));
        assert!(!is_crypto_symbol("MSFT"));
    }

    #[test]
    fn test_parse_yahoo_search_json() {
        let json = r#"{
            "quotes": [
                {
                    "symbol": "AAPL",
                    "shortname": "Apple Inc.",
                    "longname": "Apple Inc.",
                    "exchDisp": "NASDAQ",
                    "typeDisp": "Equity"
                },
                {
                    "symbol": "BTC-USD",
                    "shortname": "Bitcoin USD",
                    "longname": "Bitcoin USD",
                    "exchDisp": "CCC",
                    "typeDisp": "Cryptocurrency"
                }
            ]
        }"#;

        let results = parse_yahoo_search_json(json).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].symbol, "AAPL");
        assert_eq!(results[0].name, "Apple Inc.");
        assert_eq!(results[0].exchange, Some("NASDAQ".to_string()));
        assert_eq!(results[0].quote_type, Some("Equity".to_string()));
        assert_eq!(results[1].symbol, "BTC-USD");
    }
}
