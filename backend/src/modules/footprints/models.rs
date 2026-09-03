use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct Footprint {
    pub id: i64,
    pub owner_id: String,
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

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateFootprintRequest {
    pub title: String,
    pub latitude: f64,
    pub longitude: f64,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: String,
    pub departure_date: Option<String>,
    pub spend_amount: Option<String>,
    pub spend_currency: Option<String>,
    pub rating: Option<i64>,
    pub category: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateFootprintRequest {
    pub title: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub country: Option<String>,
    pub city: Option<String>,
    pub address: Option<String>,
    pub arrival_date: Option<String>,
    pub departure_date: Option<String>,
    pub spend_amount: Option<String>,
    pub spend_currency: Option<String>,
    pub rating: Option<i64>,
    pub category: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FootprintCurrencySpend {
    pub currency: String,
    pub currency_exponent: i64,
    pub total_spend_minor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FootprintsSummary {
    pub total_places: i64,
    pub total_countries: i64,
    pub total_days_traveled: i64,
    pub spend_by_currency: Vec<FootprintCurrencySpend>,
}

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
