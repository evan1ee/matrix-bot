use serde::{Deserialize, Serialize};

use crate::domain::habits::HabitWithStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HabitTrend {
    Improving,
    Stable,
    AtRisk,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HabitConsistencyItem {
    pub habit_id: i64,
    pub name: String,
    pub weekly_completion_rate_pct: f64,
    pub current_streak: i64,
    pub longest_streak: i64,
    pub trend: HabitTrend,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HabitConsistencyReport {
    pub total_active: usize,
    pub average_weekly_rate_pct: f64,
    pub items: Vec<HabitConsistencyItem>,
    pub slipping_habits: Vec<String>,
    pub top_streaks: Vec<String>,
}

pub fn evaluate_habit_consistency(habits: &[HabitWithStatus]) -> HabitConsistencyReport {
    let mut items = Vec::with_capacity(habits.len());
    let mut slipping_habits = Vec::new();
    let mut top_streaks = Vec::new();
    let mut total_rate_sum = 0.0_f64;

    for h in habits {
        let completions = h.last_7_days.iter().filter(|d| d.completed).count();
        let weekly_completion_rate_pct = (completions as f64 / 7.0) * 100.0;
        total_rate_sum += weekly_completion_rate_pct;

        let missing_last_2_days = if h.last_7_days.len() >= 2 {
            let mut sorted_days = h.last_7_days.clone();
            sorted_days.sort_by(|a, b| a.date.cmp(&b.date));
            let n = sorted_days.len();
            !sorted_days[n - 1].completed && !sorted_days[n - 2].completed
        } else {
            false
        };

        let trend = if weekly_completion_rate_pct >= 85.0 {
            HabitTrend::Improving
        } else if weekly_completion_rate_pct < 50.0 || missing_last_2_days {
            HabitTrend::AtRisk
        } else {
            HabitTrend::Stable
        };

        if trend == HabitTrend::AtRisk {
            slipping_habits.push(h.habit.name.clone());
        }
        if h.current_streak >= 7 {
            top_streaks.push(h.habit.name.clone());
        }

        items.push(HabitConsistencyItem {
            habit_id: h.habit.id,
            name: h.habit.name.clone(),
            weekly_completion_rate_pct,
            current_streak: h.current_streak,
            longest_streak: h.longest_streak,
            trend,
        });
    }

    let total_active = habits.len();
    let average_weekly_rate_pct = if total_active > 0 {
        total_rate_sum / total_active as f64
    } else {
        0.0
    };

    HabitConsistencyReport {
        total_active,
        average_weekly_rate_pct,
        items,
        slipping_habits,
        top_streaks,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::habits::{DayStatus, Habit};

    fn sample_habit(
        id: i64,
        name: &str,
        streak: i64,
        completions_last_7: &[bool],
    ) -> HabitWithStatus {
        let last_7_days = completions_last_7
            .iter()
            .enumerate()
            .map(|(i, &completed)| DayStatus {
                date: format!("2026-10-{:02}", 12 + i),
                completed,
            })
            .collect();

        HabitWithStatus {
            habit: Habit {
                id,
                owner_id: AccountId::from_seed("user"),
                name: name.to_string(),
                description: None,
                frequency: "daily".to_string(),
                category: "Health".to_string(),
                target_days_per_week: 7,
                archived: false,
                created_at: None,
                updated_at: None,
            },
            completed_today: completions_last_7.last().copied().unwrap_or(false),
            current_streak: streak,
            longest_streak: streak.max(10),
            total_completions: 50,
            last_7_days,
        }
    }

    #[test]
    fn test_perfect_streak() {
        let habits = vec![sample_habit(1, "Workout", 7, &[true; 7])];

        let report = evaluate_habit_consistency(&habits);
        assert_eq!(report.total_active, 1);
        assert!((report.average_weekly_rate_pct - 100.0).abs() < 1e-4);
        assert_eq!(report.items.len(), 1);

        let item = &report.items[0];
        assert_eq!(item.habit_id, 1);
        assert_eq!(item.name, "Workout");
        assert!((item.weekly_completion_rate_pct - 100.0).abs() < 1e-4);
        assert_eq!(item.trend, HabitTrend::Improving);
        assert_eq!(report.top_streaks, vec!["Workout".to_string()]);
        assert!(report.slipping_habits.is_empty());
    }

    #[test]
    fn test_slipping_habit_low_rate() {
        // 2/7 days = 28.57% < 50% -> AtRisk
        let habits = vec![sample_habit(
            1,
            "Meditation",
            0,
            &[true, true, false, false, false, false, false],
        )];

        let report = evaluate_habit_consistency(&habits);
        assert_eq!(report.items[0].trend, HabitTrend::AtRisk);
        assert_eq!(report.slipping_habits, vec!["Meditation".to_string()]);
        assert!(report.top_streaks.is_empty());
    }

    #[test]
    fn test_slipping_habit_missed_last_2_days() {
        // 5/7 days = 71.4% (would normally be Stable since >= 50% and < 85%),
        // but missed the last 2 days -> AtRisk!
        let habits = vec![sample_habit(
            1,
            "Reading",
            0,
            &[true, true, true, true, true, false, false],
        )];

        let report = evaluate_habit_consistency(&habits);
        assert_eq!(report.items[0].trend, HabitTrend::AtRisk);
        assert_eq!(report.slipping_habits, vec!["Reading".to_string()]);
    }

    #[test]
    fn test_empty_habits() {
        let report = evaluate_habit_consistency(&[]);
        assert_eq!(report.total_active, 0);
        assert_eq!(report.average_weekly_rate_pct, 0.0);
        assert!(report.items.is_empty());
        assert!(report.slipping_habits.is_empty());
        assert!(report.top_streaks.is_empty());
    }

    #[test]
    fn test_mixed_trends() {
        let habits = vec![
            // 7/7 completions -> Improving
            sample_habit(1, "Workout", 10, &[true; 7]),
            // 5/7 completions with last 2 days done -> Stable
            sample_habit(2, "Water", 2, &[false, false, true, true, true, true, true]),
            // 2/7 completions -> AtRisk
            sample_habit(3, "Journal", 0, &[false, false, false, false, false, true, true]),
        ];

        let report = evaluate_habit_consistency(&habits);
        assert_eq!(report.total_active, 3);
        assert_eq!(report.items[0].trend, HabitTrend::Improving);
        assert_eq!(report.items[1].trend, HabitTrend::Stable);
        assert_eq!(report.items[2].trend, HabitTrend::AtRisk);

        assert_eq!(report.top_streaks, vec!["Workout".to_string()]);
        assert_eq!(report.slipping_habits, vec!["Journal".to_string()]);
    }
}
