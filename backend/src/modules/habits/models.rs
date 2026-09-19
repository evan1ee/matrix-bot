use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashSet;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq, ToSchema)]
pub struct Habit {
    pub id: i64,
    pub owner_id: String,
    pub name: String,
    pub description: Option<String>,
    pub frequency: String,
    pub category: String,
    pub target_days_per_week: i64,
    pub archived: bool,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq)]
pub struct HabitLog {
    pub id: i64,
    pub habit_id: i64,
    pub owner_id: String,
    pub log_date: String,
    pub completed: bool,
    pub note: Option<String>,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct HabitWithStatus {
    #[serde(flatten)]
    pub habit: Habit,
    pub completed_today: bool,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub total_completions: i64,
    pub last_7_days: Vec<DayStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct DayStatus {
    pub date: String,
    pub completed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq, Eq, ToSchema)]
pub struct HeatmapDay {
    pub date: String,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct HabitStatsSummary {
    pub total_active_habits: i64,
    pub completed_today: i64,
    pub completion_rate_today_percent: i64,
    pub best_streak: i64,
    pub total_all_time_completions: i64,
}

/// Computes (current_streak, longest_streak) from a set of completed YYYY-MM-DD date strings.
pub fn calculate_streaks(completed_dates: &[String], today_str: &str) -> (i64, i64) {
    if completed_dates.is_empty() {
        return (0, 0);
    }

    let today = match NaiveDate::parse_from_str(today_str, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return (0, 0),
    };

    let date_set: HashSet<NaiveDate> = completed_dates
        .iter()
        .filter_map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        .collect();

    // 1. Current streak
    let mut current_streak = 0;
    let mut check_date = if date_set.contains(&today) {
        today
    } else {
        today - Duration::days(1)
    };

    while date_set.contains(&check_date) {
        current_streak += 1;
        check_date = check_date - Duration::days(1);
    }

    // 2. Longest streak
    let mut sorted_dates: Vec<NaiveDate> = date_set.into_iter().collect();
    sorted_dates.sort();

    let mut longest_streak = 0;
    let mut running_streak = 0;
    let mut prev_date: Option<NaiveDate> = None;

    for d in sorted_dates {
        match prev_date {
            Some(p) if d == p + Duration::days(1) => {
                running_streak += 1;
            }
            _ => {
                running_streak = 1;
            }
        }
        if running_streak > longest_streak {
            longest_streak = running_streak;
        }
        prev_date = Some(d);
    }

    (current_streak, longest_streak)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_streaks() {
        let dates = vec![
            "2026-08-30".to_string(),
            "2026-08-31".to_string(),
            "2026-09-01".to_string(),
            "2026-09-02".to_string(),
            "2026-09-03".to_string(),
        ];
        let (current, longest) = calculate_streaks(&dates, "2026-09-03");
        assert_eq!(current, 5);
        assert_eq!(longest, 5);

        // If today is not done yet, streak from yesterday is preserved
        let (current_prev, longest_prev) = calculate_streaks(&dates, "2026-09-04");
        assert_eq!(current_prev, 5);
        assert_eq!(longest_prev, 5);

        // Gap breaks current streak
        let (current_gap, longest_gap) = calculate_streaks(&dates, "2026-09-06");
        assert_eq!(current_gap, 0);
        assert_eq!(longest_gap, 5);
    }
}
