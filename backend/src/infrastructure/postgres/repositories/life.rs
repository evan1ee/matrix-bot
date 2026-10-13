use chrono::NaiveDate;
use sqlx::PgPool;

use crate::domain::accounts::AccountId;
use crate::domain::life::DayLogInput;
use crate::domain::life::{LifeDayLog, LifeProfile, LifeProfileStats, LifeYearView};

pub async fn get_profile(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<Option<LifeProfile>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT owner_id, birth_date, expected_lifespan_years, created_at, updated_at
        FROM life_profiles
        WHERE owner_id = $1
        "#,
        owner_id.as_uuid()
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| LifeProfile {
        owner_id: AccountId::from_uuid(r.owner_id),
        birth_date: r.birth_date,
        expected_lifespan_years: r.expected_lifespan_years,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

pub async fn upsert_profile(
    conn: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
    birth_date: &str,
    expected_lifespan_years: i64,
) -> Result<LifeProfile, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        INSERT INTO life_profiles (owner_id, birth_date, expected_lifespan_years, created_at, updated_at)
        VALUES ($1, $2, $3, NOW()::text, NOW()::text)
        ON CONFLICT(owner_id) DO UPDATE SET
            birth_date = excluded.birth_date,
            expected_lifespan_years = excluded.expected_lifespan_years,
            updated_at = NOW()::text
        RETURNING owner_id, birth_date, expected_lifespan_years, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        birth_date,
        expected_lifespan_years
    )
    .fetch_one(conn)
    .await?;

    Ok(LifeProfile {
        owner_id: AccountId::from_uuid(row.owner_id),
        birth_date: row.birth_date,
        expected_lifespan_years: row.expected_lifespan_years,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn get_day_log(
    pool: &PgPool,
    owner_id: &AccountId,
    log_date: &str,
) -> Result<Option<LifeDayLog>, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        SELECT id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
        FROM life_day_logs
        WHERE owner_id = $1 AND log_date = $2
        "#,
        owner_id.as_uuid(),
        log_date
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| LifeDayLog {
        id: r.id,
        owner_id: AccountId::from_uuid(r.owner_id),
        log_date: r.log_date,
        mood: r.mood,
        weather: r.weather,
        activities: r.activities,
        note: r.note,
        time_allocation: r.time_allocation,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }))
}

pub async fn upsert_day_log(
    conn: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
    log_date: &str,
    input: DayLogInput<'_>,
) -> Result<LifeDayLog, sqlx::Error> {
    let DayLogInput {
        mood,
        weather,
        activities,
        note,
        time_allocation,
    } = input;
    let row = sqlx::query!(
        r#"
        INSERT INTO life_day_logs (owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, NOW()::text, NOW()::text)
        ON CONFLICT(owner_id, log_date) DO UPDATE SET
            mood = excluded.mood,
            weather = excluded.weather,
            activities = excluded.activities,
            note = excluded.note,
            time_allocation = COALESCE(excluded.time_allocation, life_day_logs.time_allocation),
            updated_at = NOW()::text
        RETURNING id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        log_date,
        mood,
        weather,
        activities,
        note,
        time_allocation
    )
    .fetch_one(conn)
    .await?;

    Ok(LifeDayLog {
        id: row.id,
        owner_id: AccountId::from_uuid(row.owner_id),
        log_date: row.log_date,
        mood: row.mood,
        weather: row.weather,
        activities: row.activities,
        note: row.note,
        time_allocation: row.time_allocation,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn update_time_allocation(
    conn: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
    log_date: &str,
    time_allocation: &str,
) -> Result<LifeDayLog, sqlx::Error> {
    let row = sqlx::query!(
        r#"
        INSERT INTO life_day_logs (owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at)
        VALUES ($1, $2, '', NULL, NULL, NULL, $3, NOW()::text, NOW()::text)
        ON CONFLICT(owner_id, log_date) DO UPDATE SET
            time_allocation = excluded.time_allocation,
            updated_at = NOW()::text
        RETURNING id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        log_date,
        time_allocation
    )
    .fetch_one(conn)
    .await?;

    Ok(LifeDayLog {
        id: row.id,
        owner_id: AccountId::from_uuid(row.owner_id),
        log_date: row.log_date,
        mood: row.mood,
        weather: row.weather,
        activities: row.activities,
        note: row.note,
        time_allocation: row.time_allocation,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub async fn delete_day_log(
    conn: impl sqlx::PgExecutor<'_>,
    owner_id: &AccountId,
    log_date: &str,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query!(
        r#"
        DELETE FROM life_day_logs
        WHERE owner_id = $1 AND log_date = $2
        "#,
        owner_id.as_uuid(),
        log_date
    )
    .execute(conn)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn get_year_view(
    pool: &PgPool,
    owner_id: &AccountId,
    year: i64,
) -> Result<LifeYearView, sqlx::Error> {
    let prefix = format!("{year}-%");
    let rows = sqlx::query!(
        r#"
        SELECT id, owner_id, log_date, mood, weather, activities, note, time_allocation, created_at, updated_at
        FROM life_day_logs
        WHERE owner_id = $1 AND log_date LIKE $2
        ORDER BY log_date ASC
        "#,
        owner_id.as_uuid(),
        prefix
    )
    .fetch_all(pool)
    .await?;

    let total_logged_days = rows.len() as i64;
    let logs = rows
        .into_iter()
        .map(|r| LifeDayLog {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id),
            log_date: r.log_date,
            mood: r.mood,
            weather: r.weather,
            activities: r.activities,
            note: r.note,
            time_allocation: r.time_allocation,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect();

    Ok(LifeYearView {
        year,
        logs,
        total_logged_days,
    })
}

pub async fn get_stats(
    pool: &PgPool,
    owner_id: &AccountId,
    today_date_str: &str,
) -> Result<LifeProfileStats, sqlx::Error> {
    let profile = get_profile(pool, owner_id).await?;

    let total_logged_row = sqlx::query!(
        r#"
        SELECT COUNT(*) as "count!"
        FROM life_day_logs
        WHERE owner_id = $1 AND mood != ''
        "#,
        owner_id.as_uuid()
    )
    .fetch_one(pool)
    .await?;
    let total_logged: i64 = total_logged_row.count;

    let dominant_mood_row = sqlx::query!(
        r#"
        SELECT mood
        FROM life_day_logs
        WHERE owner_id = $1 AND mood != ''
        GROUP BY mood
        ORDER BY COUNT(*) DESC
        LIMIT 1
        "#,
        owner_id.as_uuid()
    )
    .fetch_optional(pool)
    .await?;

    let dominant_mood = dominant_mood_row.map(|r| r.mood);

    let allocations = sqlx::query!(
        r#"
        SELECT time_allocation
        FROM life_day_logs
        WHERE owner_id = $1 AND time_allocation IS NOT NULL
        "#,
        owner_id.as_uuid()
    )
    .fetch_all(pool)
    .await?;

    let mut total_deep_work_hours_acc: f64 = 0.0;
    let mut total_sleep_hours_acc: f64 = 0.0;
    let mut days_with_sleep: i64 = 0;

    for row in allocations {
        if let Some(ref alloc_json) = row.time_allocation {
            if let Ok(blocks) = serde_json::from_str::<Vec<Option<String>>>(alloc_json) {
                let block_hours = if blocks.len() <= 96 { 0.25 } else { 5.0 / 60.0 };
                let mut day_sleep_hours: f64 = 0.0;
                for cat in blocks.into_iter().flatten() {
                    if cat == "deep_work" {
                        total_deep_work_hours_acc += block_hours;
                    } else if cat == "sleep" {
                        day_sleep_hours += block_hours;
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
        (0, 0, 0.0)
    };

    Ok(LifeProfileStats {
        profile,
        age_days,
        total_expected_days,
        days_lived_percent,
        total_logged_days: total_logged,
        dominant_mood,
        total_deep_work_hours,
        average_sleep_hours,
        today_date: today_date_str.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{migrate::run_migrations_unlocked, pool::seed_accounts, pool::test_pool};

    #[tokio::test]
    async fn life_repository_roundtrip_and_stats_computation() {
        let pool = test_pool().await;
        run_migrations_unlocked(&pool).await;
        seed_accounts(&pool, &["life_repo_alice", "life_repo_bob"]).await;
        let alice = AccountId::from_seed("life_repo_alice");
        let bob = AccountId::from_seed("life_repo_bob");

        // Profile upsert & update on conflict
        let p1 = upsert_profile(&pool, &alice, "1990-01-01", 80)
            .await
            .expect("insert profile");
        assert_eq!(p1.expected_lifespan_years, 80);

        let p2 = upsert_profile(&pool, &alice, "1990-01-01", 90)
            .await
            .expect("update profile");
        assert_eq!(p2.expected_lifespan_years, 90);

        // Day log upsert & time allocation preservation on conflict
        let alloc_15m = serde_json::to_string(&vec![
            Some("sleep"),
            Some("sleep"),
            Some("sleep"),
            Some("sleep"),
            Some("deep_work"),
            Some("deep_work"),
        ])
        .unwrap();

        update_time_allocation(&pool, &alice, "2026-03-10", &alloc_15m)
            .await
            .expect("set time allocation first");

        // Upserting mood with time_allocation: None preserves existing time_allocation via COALESCE
        let d1 = upsert_day_log(
            &pool,
            &alice,
            "2026-03-10",
            DayLogInput {
                mood: "good",
                weather: Some("cloudy"),
                activities: Some("reading"),
                note: Some("Calm day"),
                time_allocation: None,
            },
        )
        .await
        .expect("upsert day log");
        assert_eq!(d1.time_allocation.as_deref(), Some(alloc_15m.as_str()));

        // Stats computation: 4 blocks of sleep (4 * 0.25h = 1.0h) and 2 blocks of deep_work (2 * 0.25h = 0.5h)
        let stats = get_stats(&pool, &alice, "2026-03-10").await.expect("stats");
        assert_eq!(stats.total_logged_days, 1);
        assert_eq!(stats.dominant_mood.as_deref(), Some("good"));
        assert!((stats.average_sleep_hours - 1.0).abs() < f64::EPSILON);
        assert!((stats.total_deep_work_hours - 0.5).abs() < f64::EPSILON);

        // Owner isolation: Bob has empty stats and year view
        let bob_stats = get_stats(&pool, &bob, "2026-03-10")
            .await
            .expect("bob stats");
        assert_eq!(bob_stats.total_logged_days, 0);
        assert!(bob_stats.profile.is_none());

        let year = get_year_view(&pool, &alice, 2026).await.expect("year view");
        assert_eq!(year.logs.len(), 1);

        assert!(delete_day_log(&pool, &alice, "2026-03-10")
            .await
            .expect("delete"));
        assert!(get_day_log(&pool, &alice, "2026-03-10")
            .await
            .expect("get after delete")
            .is_none());
    }
}
