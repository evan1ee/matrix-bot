use crate::db::DbPool;
use chrono::{Duration, NaiveDate};
use sqlx::SqlitePool;
use std::collections::HashSet;

use super::models::{
    calculate_streaks, DayStatus, Habit, HabitStatsSummary, HabitWithStatus, HeatmapDay,
};

pub struct HabitsRepository;

impl HabitsRepository {
    pub async fn create_habit(
        pool: &DbPool,
        owner_id: &str,
        name: &str,
        description: Option<&str>,
        frequency: &str,
        category: &str,
        target_days_per_week: i64,
    ) -> Result<Habit, sqlx::Error> {
        let habit = crate::db_fetch_one!(
            pool,
            Habit,
            r#"
            INSERT INTO habits (owner_id, name, description, frequency, category, target_days_per_week, archived)
            VALUES (?, ?, ?, ?, ?, ?, FALSE)
            RETURNING id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
            "#,
            owner_id,
            name,
            description,
            frequency,
            category,
            target_days_per_week
        )?;

        Ok(habit)
    }

    pub async fn find_by_id(
        pool: &DbPool,
        id: i64,
        owner_id: &str,
    ) -> Result<Option<Habit>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            Habit,
            r#"
            SELECT id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
            FROM habits
            WHERE id = ? AND owner_id = ?
            "#,
            id,
            owner_id
        )
    }

    pub async fn find_by_name(
        pool: &DbPool,
        name: &str,
        owner_id: &str,
    ) -> Result<Option<Habit>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            Habit,
            r#"
            SELECT id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
            FROM habits
            WHERE LOWER(name) = LOWER(?) AND owner_id = ? AND archived = FALSE
            LIMIT 1
            "#,
            name,
            owner_id
        )
    }

    pub async fn find_all_by_owner(
        pool: &DbPool,
        owner_id: &str,
    ) -> Result<Vec<Habit>, sqlx::Error> {
        crate::db_fetch_all!(
            pool,
            Habit,
            r#"
            SELECT id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
            FROM habits
            WHERE owner_id = ? AND archived = FALSE
            ORDER BY id ASC
            "#,
            owner_id
        )
    }

    pub async fn find_all_with_status(
        pool: &DbPool,
        owner_id: &str,
        today_str: &str,
    ) -> Result<Vec<HabitWithStatus>, sqlx::Error> {
        let habits = Self::find_all_by_owner(pool, owner_id).await?;

        let today_date = NaiveDate::parse_from_str(today_str, "%Y-%m-%d")
            .unwrap_or_else(|_| chrono::Utc::now().naive_utc().date());

        let mut results = Vec::with_capacity(habits.len());

        for habit in habits {
            let log_rows = crate::db_scalar_all!(
                pool,
                String,
                r#"
                SELECT log_date
                FROM habit_logs
                WHERE habit_id = ? AND owner_id = ? AND completed = TRUE
                ORDER BY log_date ASC
                "#,
                habit.id,
                owner_id
            )?;

            let date_set: HashSet<String> = log_rows.iter().cloned().collect();
            let (current_streak, longest_streak) = calculate_streaks(&log_rows, today_str);
            let completed_today = date_set.contains(today_str);
            let total_completions = log_rows.len() as i64;

            // Generate trailing 7 days
            let mut last_7_days = Vec::with_capacity(7);
            for i in (0..7).rev() {
                let d = today_date - Duration::days(i);
                let d_str = d.format("%Y-%m-%d").to_string();
                last_7_days.push(DayStatus {
                    completed: date_set.contains(&d_str),
                    date: d_str,
                });
            }

            results.push(HabitWithStatus {
                habit,
                completed_today,
                current_streak,
                longest_streak,
                total_completions,
                last_7_days,
            });
        }

        Ok(results)
    }

    pub async fn update_habit(
        pool: &DbPool,
        id: i64,
        owner_id: &str,
        name: &str,
        description: Option<&str>,
        frequency: &str,
        category: &str,
        target_days_per_week: i64,
    ) -> Result<Option<Habit>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            Habit,
            r#"
            UPDATE habits
            SET name = ?, description = ?, frequency = ?, category = ?, target_days_per_week = ?, updated_at = CURRENT_TIMESTAMP
            WHERE id = ? AND owner_id = ?
            RETURNING id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
            "#,
            name,
            description,
            frequency,
            category,
            target_days_per_week,
            id,
            owner_id
        )
    }

    pub async fn delete_habit(pool: &DbPool, id: i64, owner_id: &str) -> Result<bool, sqlx::Error> {
        let rows = crate::db_execute!(
            pool,
            "DELETE FROM habits WHERE id = ? AND owner_id = ?",
            id,
            owner_id
        )?;

        Ok(rows > 0)
    }

    pub async fn toggle_log(
        pool: &DbPool,
        habit_id: i64,
        owner_id: &str,
        log_date: &str,
        note: Option<&str>,
    ) -> Result<bool, sqlx::Error> {
        // Check existing log
        let existing = crate::db_scalar_optional!(
            pool,
            bool,
            r#"
            SELECT completed
            FROM habit_logs
            WHERE habit_id = ? AND owner_id = ? AND log_date = ?
            "#,
            habit_id,
            owner_id,
            log_date
        )?;

        match existing {
            Some(completed) => {
                let new_state = !completed;
                crate::db_execute!(
                    pool,
                    r#"
                    UPDATE habit_logs
                    SET completed = ?
                    WHERE habit_id = ? AND owner_id = ? AND log_date = ?
                    "#,
                    new_state,
                    habit_id,
                    owner_id,
                    log_date
                )?;
                Ok(new_state)
            }
            None => {
                crate::db_execute!(
                    pool,
                    r#"
                    INSERT INTO habit_logs (habit_id, owner_id, log_date, completed, note)
                    VALUES (?, ?, ?, 1, ?)
                    "#,
                    habit_id,
                    owner_id,
                    log_date,
                    note
                )?;
                Ok(true)
            }
        }
    }

    pub async fn get_heatmap(
        pool: &DbPool,
        owner_id: &str,
    ) -> Result<Vec<HeatmapDay>, sqlx::Error> {
        crate::db_fetch_all!(
            pool,
            HeatmapDay,
            r#"
            SELECT log_date as date, COUNT(*) as count
            FROM habit_logs
            WHERE owner_id = ? AND completed = TRUE
            GROUP BY log_date
            ORDER BY log_date ASC
            "#,
            owner_id
        )
    }

    pub async fn get_summary_stats(
        pool: &DbPool,
        owner_id: &str,
        today_str: &str,
    ) -> Result<HabitStatsSummary, sqlx::Error> {
        let habits = Self::find_all_with_status(pool, owner_id, today_str).await?;
        let total_active_habits = habits.len() as i64;
        let completed_today = habits.iter().filter(|h| h.completed_today).count() as i64;
        let completion_rate_today_percent = if total_active_habits > 0 {
            (completed_today * 100) / total_active_habits
        } else {
            0
        };

        let best_streak = habits.iter().map(|h| h.current_streak).max().unwrap_or(0);
        let total_all_time_completions: i64 = habits.iter().map(|h| h.total_completions).sum();

        Ok(HabitStatsSummary {
            total_active_habits,
            completed_today,
            completion_rate_today_percent,
            best_streak,
            total_all_time_completions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_db() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db::schema_habits::create_tables(&pool)
            .await
            .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_habits_crud_and_streaks() {
        let pool = setup_db().await;
        let habit = HabitsRepository::create_habit(
            &pool,
            "user_1",
            "Gym Workout",
            Some("Strength training"),
            "daily",
            "fitness",
            7,
        )
        .await
        .unwrap();

        assert_eq!(habit.name, "Gym Workout");
        assert_eq!(habit.category, "fitness");

        // Toggle logs
        let s1 = HabitsRepository::toggle_log(&pool, habit.id, "user_1", "2026-09-02", None)
            .await
            .unwrap();
        assert!(s1);

        let s2 = HabitsRepository::toggle_log(&pool, habit.id, "user_1", "2026-09-03", None)
            .await
            .unwrap();
        assert!(s2);

        let items = HabitsRepository::find_all_with_status(&pool, "user_1", "2026-09-03")
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert!(items[0].completed_today);
        assert_eq!(items[0].current_streak, 2);
        assert_eq!(items[0].longest_streak, 2);

        // Heatmap
        let heatmap = HabitsRepository::get_heatmap(&pool, "user_1")
            .await
            .unwrap();
        assert_eq!(heatmap.len(), 2);

        // Stats
        let stats = HabitsRepository::get_summary_stats(&pool, "user_1", "2026-09-03")
            .await
            .unwrap();
        assert_eq!(stats.total_active_habits, 1);
        assert_eq!(stats.completed_today, 1);
        assert_eq!(stats.completion_rate_today_percent, 100);
        assert_eq!(stats.best_streak, 2);
    }
}
