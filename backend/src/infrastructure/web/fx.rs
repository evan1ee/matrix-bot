//! Foreign exchange (FX) rates HTTP fetching and caching service.
//!
//! Fetches daily reference rates from the European Central Bank (ECB) eurofxref feed
//! with in-memory caching and fallback to Yahoo Finance rates.

use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::domain::finance::fx::{baseline_rates, parse_ecb_xml};

const ECB_URL: &str = "https://www.ecb.europa.eu/stats/eurofxref/eurofxref-daily.xml";
const CACHE_TTL: Duration = Duration::from_secs(6 * 3600); // 6 hours

#[derive(Clone, Debug)]
pub struct FxCache {
    pub rates_eur_base: HashMap<String, f64>,
    pub fetched_at: DateTime<Utc>,
}

static FX_CACHE: RwLock<Option<FxCache>> = RwLock::const_new(None);

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
        let cache_guard = FX_CACHE.read().await;
        if let Some(ref cache) = *cache_guard {
            let age = Utc::now() - cache.fetched_at;
            if age.num_seconds() < CACHE_TTL.as_secs() as i64 {
                return cache.rates_eur_base.clone();
            }
        }
    }

    info!("FX cache stale or empty, refreshing from ECB...");
    let new_rates = match fetch_ecb_rates(client).await {
        Ok(rates) => {
            info!("Successfully fetched {} rates from ECB", rates.len());
            rates
        }
        Err(err) => {
            warn!(
                "ECB rate refresh failed: {}. Retaining previous/fallback rates.",
                err
            );
            let cache_guard = FX_CACHE.read().await;
            if let Some(ref cache) = *cache_guard {
                return cache.rates_eur_base.clone();
            }
            baseline_rates()
        }
    };

    let mut cache_guard = FX_CACHE.write().await;
    *cache_guard = Some(FxCache {
        rates_eur_base: new_rates.clone(),
        fetched_at: Utc::now(),
    });

    new_rates
}

static DEFAULT_FX_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

/// Retrieves the current FX rates against EUR using a shared internal HTTP client.
pub async fn get_cached_rates() -> HashMap<String, f64> {
    let client = DEFAULT_FX_CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .use_rustls_tls()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to build FX HTTP client")
    });
    get_rates(client).await
}
