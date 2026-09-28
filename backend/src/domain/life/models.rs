use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::domain::accounts::AccountId;

/// Profile containing birth date and life expectancy for timeline calculations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifeProfile {
    pub owner_id: AccountId,
    pub birth_date: String,
    pub expected_lifespan_years: i64,
    pub created_at: String,
    pub updated_at: String,
}

/// A logged day record with mood, weather, activities, notes and time allocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifeDayLog {
    pub id: i64,
    pub owner_id: AccountId,
    pub log_date: String,
    pub mood: String,
    pub weather: Option<String>,
    pub activities: Option<String>,
    pub note: Option<String>,
    pub time_allocation: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// A single day on the multi-year life horizon grid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifeHorizonDay {
    pub date: String,
    pub mood: String,
    pub time_allocation: Option<String>,
    pub has_activity: bool,
    pub activity_count: i64,
}

/// Calculated profile statistics including age in days and percentage of expected life lived.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// A full calendar year view with day logs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifeYearView {
    pub year: i64,
    pub logs: Vec<LifeDayLog>,
    pub total_logged_days: i64,
}

/// A single cross-module event that occurred on a specific date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub time: Option<String>,
    pub module: String,
    pub action: String,
    pub emoji: String,
    pub summary: String,
    pub detail: Option<String>,
}

/// Summary counts of cross-module activities on a date.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ActivityStats {
    pub total_events: usize,
    pub todos_created: usize,
    pub todos_completed: usize,
    pub todos_due: usize,
    pub expenses: usize,
    pub income: usize,
    pub portfolio_trades: usize,
    pub habits_done: usize,
    pub reminders_fired: usize,
    pub bookmarks_saved: usize,
    pub bills_paid: usize,
    pub footprints: usize,
}

/// Full aggregated activity feed for a given calendar day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayActivityResponse {
    pub date: String,
    pub events: Vec<ActivityEvent>,
    pub stats: ActivityStats,
}

/// Normalizes human or numeric mood inputs into canonical labels:
/// "amazing", "good", "neutral", "low", "rough".
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

/// Validates that a string is a valid YYYY-MM-DD date.
pub fn validate_date_str(s: &str) -> Result<NaiveDate, &'static str> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d")
        .map_err(|_| "Invalid date format. Use YYYY-MM-DD.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_mood() {
        assert_eq!(normalize_mood("amazing"), Some("amazing"));
        assert_eq!(normalize_mood("5"), Some("amazing"));
        assert_eq!(normalize_mood("happy"), Some("good"));
        assert_eq!(normalize_mood("4"), Some("good"));
        assert_eq!(normalize_mood("neutral"), Some("neutral"));
        assert_eq!(normalize_mood("3"), Some("neutral"));
        assert_eq!(normalize_mood("low"), Some("low"));
        assert_eq!(normalize_mood("2"), Some("low"));
        assert_eq!(normalize_mood("rough"), Some("rough"));
        assert_eq!(normalize_mood("1"), Some("rough"));
        assert_eq!(normalize_mood("unknown"), None);
    }

    #[test]
    fn test_validate_date() {
        assert!(validate_date_str("2026-09-28").is_ok());
        assert!(validate_date_str("invalid").is_err());
        assert!(validate_date_str("2026-02-31").is_err());
    }
}
