use chrono::{Duration, NaiveDate};
use futures_util::future::join_all;
use std::collections::HashSet;
use tracing::warn;

use crate::domain::portfolio::models::Holding;
use crate::modules::briefing::models::TickerPerformance;

/// Computes multi-timeframe percentage changes from sorted historical daily points `[(date, price)]`.
pub fn compute_ticker_performance(
    symbol: &str,
    currency: &str,
    fallback_price: Option<f64>,
    points: &[(String, f64)],
) -> TickerPerformance {
    if points.is_empty() {
        return TickerPerformance {
            symbol: symbol.to_string(),
            currency: currency.to_string(),
            current_price: fallback_price.unwrap_or(0.0),
            change_1d_pct: None,
            change_1w_pct: None,
            change_1m_pct: None,
        };
    }

    let latest_point = points.last().unwrap();
    let current_price = if latest_point.1 > 0.0 {
        latest_point.1
    } else {
        fallback_price.unwrap_or(0.0)
    };

    // 1-Day change: compare with the previous day's close
    let change_1d_pct = if points.len() >= 2 {
        let prev_day_price = points[points.len() - 2].1;
        calc_pct_change(current_price, prev_day_price)
    } else {
        None
    };

    let latest_date = NaiveDate::parse_from_str(&latest_point.0, "%Y-%m-%d").ok();

    // 1-Week change: find the closest point <= 7 calendar days ago
    let change_1w_pct = if let Some(latest_d) = latest_date {
        let target_date = (latest_d - Duration::days(7))
            .format("%Y-%m-%d")
            .to_string();
        let price_1w = points
            .iter()
            .rev()
            .find(|(date, _)| date <= &target_date)
            .map(|(_, price)| *price)
            .or_else(|| {
                // If fewer than 7 days exist but at least 5 points, compare with earliest
                if points.len() >= 5 {
                    Some(points[0].1)
                } else {
                    None
                }
            });

        price_1w.and_then(|p| calc_pct_change(current_price, p))
    } else {
        None
    };

    // 1-Month change: find the earliest point in the 1-month window or closest to 30 days ago
    let change_1m_pct = if let Some(latest_d) = latest_date {
        let target_date = (latest_d - Duration::days(30))
            .format("%Y-%m-%d")
            .to_string();
        let price_1m = points
            .iter()
            .find(|(date, _)| date >= &target_date)
            .map(|(_, price)| *price)
            .or_else(|| points.first().map(|(_, p)| *p));

        price_1m.and_then(|p| calc_pct_change(current_price, p))
    } else {
        points
            .first()
            .and_then(|(_, p)| calc_pct_change(current_price, *p))
    };

    TickerPerformance {
        symbol: symbol.to_string(),
        currency: currency.to_string(),
        current_price,
        change_1d_pct,
        change_1w_pct,
        change_1m_pct,
    }
}

fn calc_pct_change(current: f64, past: f64) -> Option<f64> {
    if past > 0.0 {
        Some(((current - past) / past) * 100.0)
    } else {
        None
    }
}

/// Fetches multi-timeframe ticker performance for a symbol.
pub async fn fetch_holding_ticker_performance(
    symbol: &str,
    currency: &str,
    fallback_price: Option<f64>,
) -> TickerPerformance {
    match crate::infrastructure::web::portfolio_history_feed::fetch_historical_prices(symbol, "1m")
        .await
    {
        Ok(points) if !points.is_empty() => {
            compute_ticker_performance(symbol, currency, fallback_price, &points)
        }
        Ok(_) => compute_ticker_performance(symbol, currency, fallback_price, &[]),
        Err(err) => {
            warn!("Failed fetching 1m ticker prices for {}: {}", symbol, err);
            compute_ticker_performance(symbol, currency, fallback_price, &[])
        }
    }
}

/// Concurrently fetches performance for all holdings owned by the user.
pub async fn fetch_all_tickers_performance(holdings: &[Holding]) -> Vec<TickerPerformance> {
    let mut seen = HashSet::new();
    let mut unique_holdings = Vec::new();

    for h in holdings {
        let clean = h.symbol.trim().to_uppercase();
        if !clean.is_empty() && seen.insert(clean) {
            unique_holdings.push(h);
        }
    }

    let futures = unique_holdings.into_iter().map(|h| {
        let sym = h.symbol.clone();
        let cur = h.currency.clone();
        let fallback_price = h.current_price_minor.map(|minor| {
            let scale = 10f64.powi(h.currency_exponent.clamp(0, 8) as i32);
            minor as f64 / scale
        });

        async move { fetch_holding_ticker_performance(&sym, &cur, fallback_price).await }
    });

    join_all(futures).await
}

/// Formats a percentage change as colored Matrix HTML.
pub fn format_pct_html(pct_opt: Option<f64>) -> String {
    match pct_opt {
        Some(pct) if pct > 0.0 => format!("<font color=\"#10b981\">+{:.1}%</font>", pct),
        Some(pct) if pct < 0.0 => format!("<font color=\"#ef4444\">{:.1}%</font>", pct),
        Some(pct) => format!("<font color=\"#888888\">{:.1}%</font>", pct),
        None => "<font color=\"#888888\">--</font>".to_string(),
    }
}

/// Formats a percentage change as plain text.
pub fn format_pct_plain(pct_opt: Option<f64>) -> String {
    match pct_opt {
        Some(pct) if pct > 0.0 => format!("+{:.1}%", pct),
        Some(pct) => format!("{:.1}%", pct),
        None => "--".to_string(),
    }
}

/// Formats one ticker performance line in Matrix HTML.
pub fn format_ticker_html(t: &TickerPerformance) -> String {
    format!(
        "• <b>{}:</b> {:.2} {} (1D: {} | 1W: {} | 1M: {})",
        t.symbol,
        t.current_price,
        t.currency,
        format_pct_html(t.change_1d_pct),
        format_pct_html(t.change_1w_pct),
        format_pct_html(t.change_1m_pct),
    )
}

/// Formats one ticker performance line in plain text.
pub fn format_ticker_plain(t: &TickerPerformance) -> String {
    format!(
        "• {}: {:.2} {} (1D: {} | 1W: {} | 1M: {})",
        t.symbol,
        t.current_price,
        t.currency,
        format_pct_plain(t.change_1d_pct),
        format_pct_plain(t.change_1w_pct),
        format_pct_plain(t.change_1m_pct),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_ticker_performance_with_series() {
        let points = vec![
            ("2026-08-18".to_string(), 100.0), // 30 days ago
            ("2026-09-10".to_string(), 110.0), // 7 days ago
            ("2026-09-16".to_string(), 115.0), // Yesterday
            ("2026-09-17".to_string(), 120.0), // Today (latest)
        ];

        let res = compute_ticker_performance("NVDA", "USD", None, &points);
        assert_eq!(res.symbol, "NVDA");
        assert_eq!(res.current_price, 120.0);

        // 1D: (120 - 115) / 115 * 100 = 4.3478%
        let p_1d = res.change_1d_pct.unwrap();
        assert!((p_1d - 4.3478).abs() < 0.01);

        // 1W: (120 - 110) / 110 * 100 = 9.0909%
        let p_1w = res.change_1w_pct.unwrap();
        assert!((p_1w - 9.0909).abs() < 0.01);

        // 1M: (120 - 100) / 100 * 100 = 20.0%
        let p_1m = res.change_1m_pct.unwrap();
        assert!((p_1m - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_ticker_performance_empty() {
        let res = compute_ticker_performance("BTC", "USD", Some(65000.0), &[]);
        assert_eq!(res.symbol, "BTC");
        assert_eq!(res.current_price, 65000.0);
        assert_eq!(res.change_1d_pct, None);
        assert_eq!(res.change_1w_pct, None);
        assert_eq!(res.change_1m_pct, None);
    }

    #[test]
    fn test_formatting() {
        let t = TickerPerformance {
            symbol: "NVDA".to_string(),
            currency: "USD".to_string(),
            current_price: 120.50,
            change_1d_pct: Some(2.5),
            change_1w_pct: Some(-1.2),
            change_1m_pct: Some(15.0),
        };

        let html = format_ticker_html(&t);
        assert!(html.contains("NVDA"));
        assert!(html.contains("+2.5%"));
        assert!(html.contains("-1.2%"));
        assert!(html.contains("+15.0%"));

        let plain = format_ticker_plain(&t);
        assert!(plain.contains("NVDA: 120.50 USD"));
        assert!(plain.contains("1D: +2.5%"));
    }
}
