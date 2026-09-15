use serde::{Deserialize, Serialize};
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct LifeProfile {
    pub owner_id: String,
    pub birth_date: String,
    pub expected_lifespan_years: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct LifeDayLog {
    pub id: i64,
    pub owner_id: String,
    pub log_date: String,
    pub mood: String,
    pub weather: Option<String>,
    pub activities: Option<String>,
    pub note: Option<String>,
    pub time_allocation: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLifeProfileInput {
    pub birth_date: String,
    pub expected_lifespan_years: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLifeDayLogInput {
    #[serde(alias = "date")]
    pub log_date: String,
    pub mood: String,
    pub weather: Option<String>,
    pub activities: Option<Vec<String>>,
    pub note: Option<String>,
    pub time_allocation: Option<Vec<Option<String>>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTimeAllocationInput {
    pub time_allocation: Vec<Option<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct LifeHorizonDay {
    pub date: String,
    pub mood: String,
    pub time_allocation: Option<String>,
    pub has_activity: bool,
    pub activity_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LifeProfileStats {
    pub profile: Option<LifeProfile>,
    pub age_days: i64,
    pub total_expected_days: i64,
    pub days_lived_percent: f64,
    pub total_logged_days: i64,
    pub dominant_mood: Option<String>,
    pub total_deep_work_hours: f64,
    pub average_sleep_hours: f64,
    pub today_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifeYearView {
    pub year: i64,
    pub logs: Vec<LifeDayLog>,
    pub total_logged_days: i64,
}

pub fn normalize_mood(input: &str) -> Option<&'static str> {
    match input.trim().to_lowercase().as_str() {
        "amazing" | "rad" | "great" | "5" => Some("amazing"),
        "good" | "happy" | "4" => Some("good"),
        "neutral" | "okay" | "ok" | "calm" | "3" => Some("neutral"),
        "low" | "down" | "tired" | "2" => Some("low"),
        "rough" | "stressed" | "bad" | "1" => Some("rough"),
        _ => None,
    }
}
