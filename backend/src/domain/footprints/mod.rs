//! Pure domain types, invariants, and calculations for travel footprints.
//!
//! Depends only on std, chrono, serde, and thiserror.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

pub const MAX_TITLE_LEN: usize = 500;
pub const MAX_NOTE_LEN: usize = 500;

/// A recorded travel footprint / visited place.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    pub id: i64,
    pub owner_id: AccountId,
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub stay_duration_days: i64,
    pub spend_amount_minor: i64,
    pub spend_currency: String,
    pub spend_currency_exponent: i64,
    pub rating: i64,
    pub category: String,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Validated input for creating a footprint.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateFootprint {
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub stay_duration_days: i64,
    pub spend_amount_minor: i64,
    pub spend_currency: String,
    pub spend_currency_exponent: i64,
    pub rating: i64,
    pub category: String,
    pub note: Option<String>,
}

/// Validated input for updating an existing footprint.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateFootprint {
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub stay_duration_days: i64,
    pub spend_amount_minor: i64,
    pub spend_currency: String,
    pub spend_currency_exponent: i64,
    pub rating: i64,
    pub category: String,
    pub note: Option<String>,
}

/// Spend summary breakdown for a specific currency in footprints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootprintCurrencySpend {
    pub currency: String,
    pub currency_exponent: i64,
    pub total_spend_minor: i64,
}

/// Summary travel statistics across all recorded footprints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FootprintsSummary {
    pub total_places: i64,
    pub total_countries: i64,
    pub total_days_traveled: i64,
    pub spend_by_currency: Vec<FootprintCurrencySpend>,
}

/// Calculates stay duration in days between arrival and optional departure.
/// Defaults to at least 1 day.
pub fn calculate_stay_duration(arrival: &str, departure: Option<&str>) -> i64 {
    let Ok(arr) = NaiveDate::parse_from_str(arrival, "%Y-%m-%d") else {
        return 1;
    };
    if let Some(dep_str) = departure {
        if let Ok(dep) = NaiveDate::parse_from_str(dep_str, "%Y-%m-%d") {
            let days = (dep - arr).num_days();
            return days.max(1);
        }
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stay_duration_calculation() {
        assert_eq!(calculate_stay_duration("2026-06-01", Some("2026-06-05")), 4);
        assert_eq!(calculate_stay_duration("2026-06-01", Some("2026-06-01")), 1);
        assert_eq!(calculate_stay_duration("2026-06-05", Some("2026-06-01")), 1);
        assert_eq!(calculate_stay_duration("2026-06-01", None), 1);
        assert_eq!(calculate_stay_duration("invalid", Some("2026-06-05")), 1);
        assert_eq!(calculate_stay_duration("2026-06-01", Some("invalid")), 1);
    }
}
