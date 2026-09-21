use crate::db::DbPool;
use chrono::NaiveDate;
use sqlx::SqlitePool;

use super::models::{LifeDayLog, LifeHorizonDay, LifeProfile, LifeProfileStats, LifeYearView};

pub struct LifeRepository;

impl LifeRepository {
    pub async fn get_profile(
        pool: &DbPool,
        owner_id: &str,
    ) -> Result<Option<LifeProfile>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            LifeProfile,
            r#"
            SELECT owner_id, birth_date, expected_lifespan_years, created_at, updated_at
            FROM life_profiles
            WHERE owner_id = ?
            "#,
            owner_id
        )
    }

    pub async fn upsert_profile(
        pool: &DbPool,
        owner_id: &str,
        birth_date: &str,
        expected_lifespan_years: i64,
    ) -> Result<LifeProfile, sqlx::Error> {
        crate::db_fetch_one!(
            pool,
            LifeProfile,
            r#"
            INSERT INTO life_profiles (owner_id, birth_date, expected_lifespan_years, created_at, updated_at)
            VALUES (?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(owner_id) DO UPDATE SET
                birth_date = excluded.birth_date,
                expected_lifespan_years = excluded.expected_lifespan_years,
                updated_at = datetime('now')
            RETURNING owner_id, birth_date, expected_lifespan_years, created_at, updated_at
            "#,
            owner_id,
            birth_date,
            expected_lifespan_years
        )
    }

    pub async fn get_day_log(
        pool: &DbPool,
        owner_id: &str,
        log_date: &str,
    ) -> Result<Option<LifeDayLog>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            LifeDayLog,
            r#"
            SELECT id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
            FROM life_day_logs
            WHERE owner_id = ? AND log_date = ?
            "#,
            owner_id,
            log_date
        )
    }

    pub async fn upsert_day_log(
        pool: &DbPool,
        owner_id: &str,
        log_date: &str,
        mood: &str,
        weather: Option<&str>,
        activities: Option<&str>,
        note: Option<&str>,
        time_allocation: Option<&str>,
    ) -> Result<LifeDayLog, sqlx::Error> {
        crate::db_fetch_one!(
            pool,
            LifeDayLog,
            r#"
            INSERT INTO life_day_logs (owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'), datetime('now'))
            ON CONFLICT(owner_id, log_date) DO UPDATE SET
                mood = excluded.mood,
                weather = excluded.weather,
                activities = excluded.activities,
                note = excluded.note,
                time_allocation = COALESCE(excluded.time_allocation, life_day_logs.time_allocation),
                updated_at = datetime('now')
            RETURNING id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
            "#,
            owner_id,
            log_date,
            mood,
            weather,
            activities,
            note,
            time_allocation
        )
    }

    pub async fn update_time_allocation(
        pool: &DbPool,
        owner_id: &str,
        log_date: &str,
        time_allocation: &str,
    ) -> Result<LifeDayLog, sqlx::Error> {
        crate::db_fetch_one!(
            pool,
            LifeDayLog,
            r#"
            INSERT INTO life_day_logs (owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at)
            VALUES (?, ?, '', NULL, NULL, NULL, ?, datetime('now'), datetime('now'))
            ON CONFLICT(owner_id, log_date) DO UPDATE SET
                time_allocation = excluded.time_allocation,
                updated_at = datetime('now')
            RETURNING id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
            "#,
            owner_id,
            log_date,
            time_allocation
        )
    }

    pub async fn delete_day_log(
        pool: &DbPool,
        owner_id: &str,
        log_date: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = crate::db_execute!(
            pool,
            r#"
            DELETE FROM life_day_logs
            WHERE owner_id = ? AND log_date = ?
            "#,
            owner_id,
            log_date
        )?;

        Ok(result > 0)
    }

    pub async fn get_year_view(
        pool: &DbPool,
        owner_id: &str,
        year: i64,
    ) -> Result<LifeYearView, sqlx::Error> {
        let prefix = format!("{year}-%");
        let logs = crate::db_fetch_all!(
            pool,
            LifeDayLog,
            r#"
            SELECT id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
            FROM life_day_logs
            WHERE owner_id = ? AND log_date LIKE ?
            ORDER BY log_date ASC
            "#,
            owner_id,
            &prefix
        )?;

        let total_logged_days = logs.len() as i64;

        Ok(LifeYearView {
            year,
            logs,
            total_logged_days,
        })
    }

    pub async fn get_horizon_days(
        pool: &DbPool,
        owner_id: &str,
    ) -> Result<Vec<LifeHorizonDay>, sqlx::Error> {
        crate::db_fetch_all!(
            pool,
            LifeHorizonDay,
            r#"
            WITH user_activities AS (
                SELECT date(created_at) AS act_date FROM todos WHERE owner_id = ? AND created_at IS NOT NULL
                UNION ALL
                SELECT date(due_at) AS act_date FROM todos WHERE owner_id = ? AND due_at IS NOT NULL
                UNION ALL
                SELECT date(occurred_at) AS act_date FROM transactions WHERE owner_id = ? AND occurred_at IS NOT NULL
                UNION ALL
                SELECT date(created_at) AS act_date FROM portfolio_audit_log WHERE owner_id = ? AND created_at IS NOT NULL
                UNION ALL
                SELECT log_date AS act_date FROM habit_logs WHERE owner_id = ? AND log_date IS NOT NULL
                UNION ALL
                SELECT date(trigger_at) AS act_date FROM reminders WHERE owner_id = ? AND trigger_at IS NOT NULL
                UNION ALL
                SELECT date(created_at) AS act_date FROM bookmarks WHERE owner_id = ? AND created_at IS NOT NULL
                UNION ALL
                SELECT arrival_date AS act_date FROM footprints WHERE owner_id = ? AND arrival_date IS NOT NULL
            ),
            activity_counts AS (
                SELECT act_date, COUNT(*) AS cnt 
                FROM user_activities 
                WHERE act_date IS NOT NULL AND act_date != ''
                GROUP BY act_date
            ),
            all_dates AS (
                SELECT log_date AS d_date FROM life_day_logs WHERE owner_id = ?
                UNION
                SELECT act_date AS d_date FROM activity_counts
            )
            SELECT 
                all_dates.d_date AS date,
                COALESCE(l.mood, '') AS mood,
                l.time_allocation AS time_allocation,
                (COALESCE(ac.cnt, 0) > 0) AS has_activity,
                COALESCE(ac.cnt, 0) AS activity_count
            FROM all_dates
            LEFT JOIN life_day_logs l ON l.owner_id = ? AND l.log_date = all_dates.d_date
            LEFT JOIN activity_counts ac ON ac.act_date = all_dates.d_date
            WHERE all_dates.d_date IS NOT NULL AND all_dates.d_date != ''
            ORDER BY all_dates.d_date ASC
            "#,
            owner_id,
            owner_id,
            owner_id,
            owner_id,
            owner_id,
            owner_id,
            owner_id,
            owner_id,
            owner_id,
            owner_id
        )
    }

    pub async fn get_stats(
        pool: &DbPool,
        owner_id: &str,
        today_date_str: &str,
    ) -> Result<LifeProfileStats, sqlx::Error> {
        let profile = Self::get_profile(pool, owner_id).await?;

        let total_logged: (i64,) = crate::db_fetch_one!(
            pool,
            _,
            r#"
            SELECT COUNT(*) FROM life_day_logs WHERE owner_id = ? AND mood != ''
            "#,
            owner_id
        )?;

        let dominant_mood_row: Option<(String,)> = crate::db_fetch_optional!(
            pool,
            _,
            r#"
            SELECT mood FROM life_day_logs
            WHERE owner_id = ? AND mood != ''
            GROUP BY mood
            ORDER BY COUNT(*) DESC
            LIMIT 1
            "#,
            owner_id
        )?;

        let dominant_mood = dominant_mood_row.map(|r| r.0);

        let allocations: Vec<(Option<String>,)> = crate::db_fetch_all!(
            pool,
            _,
            r#"
            SELECT time_allocation FROM life_day_logs
            WHERE owner_id = ? AND time_allocation IS NOT NULL
            "#,
            owner_id
        )?;

        let mut total_deep_work_hours_acc: f64 = 0.0;
        let mut total_sleep_hours_acc: f64 = 0.0;
        let mut days_with_sleep: i64 = 0;

        for (alloc_json_opt,) in allocations {
            if let Some(ref alloc_json) = alloc_json_opt {
                if let Ok(blocks) = serde_json::from_str::<Vec<Option<String>>>(alloc_json) {
                    let block_hours = if blocks.len() <= 96 { 0.25 } else { 5.0 / 60.0 };
                    let mut day_sleep_hours: f64 = 0.0;
                    for b in blocks {
                        if let Some(cat) = b {
                            if cat == "deep_work" {
                                total_deep_work_hours_acc += block_hours;
                            } else if cat == "sleep" {
                                day_sleep_hours += block_hours;
                            }
                        }
                    }
                    if day_sleep_hours > 0.0 {
                        total_sleep_hours_acc += day_sleep_hours;
                        days_with_sleep += 1;
                    }
                }
            }
        }

        let total_deep_work_hours = (total_deep_work_hours_acc * 10.0).round() / 10.0;
        let average_sleep_hours = if days_with_sleep > 0 {
            ((total_sleep_hours_acc / days_with_sleep as f64) * 10.0).round() / 10.0
        } else {
            0.0
        };

        let (age_days, total_expected_days, days_lived_percent) = if let Some(ref p) = profile {
            if let (Ok(birth), Ok(today)) = (
                NaiveDate::parse_from_str(&p.birth_date, "%Y-%m-%d"),
                NaiveDate::parse_from_str(today_date_str, "%Y-%m-%d"),
            ) {
                let age = (today - birth).num_days().max(0);
                let total_days = (p.expected_lifespan_years as f64 * 365.25) as i64;
                let percent = if total_days > 0 {
                    ((age as f64 / total_days as f64) * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                (age, total_days, (percent * 10.0).round() / 10.0)
            } else {
                (0, p.expected_lifespan_years * 365, 0.0)
            }
        } else {
            (0, 80 * 365, 0.0)
        };

        Ok(LifeProfileStats {
            profile,
            age_days,
            total_expected_days,
            days_lived_percent,
            total_logged_days: total_logged.0,
            dominant_mood,
            total_deep_work_hours,
            average_sleep_hours,
            today_date: today_date_str.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn setup_test_db() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn test_profile_upsert_and_get() {
        let pool = setup_test_db().await;
        let owner = "user_test_1";

        assert!(LifeRepository::get_profile(&pool, owner)
            .await
            .unwrap()
            .is_none());

        let prof = LifeRepository::upsert_profile(&pool, owner, "1995-06-15", 85)
            .await
            .unwrap();
        assert_eq!(prof.birth_date, "1995-06-15");
        assert_eq!(prof.expected_lifespan_years, 85);

        // Update existing
        let updated = LifeRepository::upsert_profile(&pool, owner, "1995-06-15", 90)
            .await
            .unwrap();
        assert_eq!(updated.expected_lifespan_years, 90);
    }

    #[tokio::test]
    async fn test_day_log_crud_and_stats() {
        let pool = setup_test_db().await;
        let owner = "user_test_2";

        LifeRepository::upsert_profile(&pool, owner, "2000-01-01", 80)
            .await
            .unwrap();

        let log = LifeRepository::upsert_day_log(
            &pool,
            owner,
            "2026-09-04",
            "amazing",
            Some("sunny"),
            Some("[\"workout\",\"family\"]"),
            Some("A great productive day"),
            Some("[\"sleep\",\"sleep\",\"deep_work\",\"deep_work\"]"),
        )
        .await
        .unwrap();

        assert_eq!(log.mood, "amazing");
        assert_eq!(log.weather.as_deref(), Some("sunny"));
        assert!(log.time_allocation.is_some());

        let stats = LifeRepository::get_stats(&pool, owner, "2026-09-04")
            .await
            .unwrap();
        assert_eq!(stats.total_logged_days, 1);
        assert_eq!(stats.dominant_mood.as_deref(), Some("amazing"));
        assert_eq!(stats.total_deep_work_hours, 0.5);
        assert_eq!(stats.average_sleep_hours, 0.5);
        assert!(stats.age_days > 9000);

        // Test updating time allocation intraday
        let updated_time = LifeRepository::update_time_allocation(
            &pool,
            owner,
            "2026-09-04",
            "[\"deep_work\",\"deep_work\",\"deep_work\",\"deep_work\"]",
        )
        .await
        .unwrap();
        assert_eq!(updated_time.mood, "amazing");
        assert_eq!(
            updated_time.time_allocation.as_deref(),
            Some("[\"deep_work\",\"deep_work\",\"deep_work\",\"deep_work\"]")
        );

        let year_view = LifeRepository::get_year_view(&pool, owner, 2026)
            .await
            .unwrap();
        assert_eq!(year_view.logs.len(), 1);

        let horizon = LifeRepository::get_horizon_days(&pool, owner)
            .await
            .unwrap();
        assert_eq!(horizon.len(), 1);
        assert_eq!(horizon[0].mood, "amazing");
        assert_eq!(horizon[0].has_activity, false);

        let deleted = LifeRepository::delete_day_log(&pool, owner, "2026-09-04")
            .await
            .unwrap();
        assert!(deleted);
        assert!(LifeRepository::get_day_log(&pool, owner, "2026-09-04")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_horizon_includes_auto_logged_activity_days() {
        let pool = setup_test_db().await;
        let owner = "user_test_activity";

        // Insert a todo for a date without a life_day_log
        crate::db_execute!(&pool, r#"
            INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed, created_at)
            VALUES ('!room:matrix.org', ?, 'Alice', 'Workout task', 'medium', 0, '2026-09-10 10:00:00')
            "#, owner)
        .unwrap();

        let horizon = LifeRepository::get_horizon_days(&pool, owner)
            .await
            .unwrap();
        assert_eq!(horizon.len(), 1);
        assert_eq!(horizon[0].date, "2026-09-10");
        assert_eq!(horizon[0].mood, "");
        assert_eq!(horizon[0].has_activity, true);
        assert_eq!(horizon[0].activity_count, 1);
    }
}
