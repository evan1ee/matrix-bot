//! PostgreSQL repository for habits and habit logs.
//!
//! Compile-checked queries over `impl sqlx::PgExecutor`. Maps private `HabitRow`
//! into domain `Habit` before leaving this file.

use chrono::{Duration, NaiveDate};
use sqlx::PgExecutor;
use std::collections::HashSet;

use crate::domain::accounts::AccountId;
use crate::domain::habits::{calculate_streaks, DayStatus, Habit, HabitWithStatus, HeatmapDay};

#[derive(Debug, sqlx::FromRow)]
struct HabitRow {
    id: i64,
    owner_id: uuid::Uuid,
    name: String,
    description: Option<String>,
    frequency: String,
    category: String,
    target_days_per_week: i64,
    archived: bool,
    created_at: Option<String>,
    updated_at: Option<String>,
}

impl From<HabitRow> for Habit {
    fn from(row: HabitRow) -> Self {
        Self {
            id: row.id,
            owner_id: AccountId::from_uuid(row.owner_id),
            name: row.name,
            description: row.description,
            frequency: row.frequency,
            category: row.category,
            target_days_per_week: row.target_days_per_week,
            archived: row.archived,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

pub async fn create_habit<'e>(
    conn: impl PgExecutor<'e>,
    owner: &AccountId,
    name: &str,
    description: Option<&str>,
    frequency: &str,
    category: &str,
    target_days_per_week: i64,
) -> Result<Habit, sqlx::Error> {
    let row = sqlx::query_as!(
        HabitRow,
        r#"
        INSERT INTO habits (owner_id, name, description, frequency, category, target_days_per_week, archived)
        VALUES ($1, $2, $3, $4, $5, $6, FALSE)
        RETURNING id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
        "#,
        owner.as_uuid(),
        name,
        description,
        frequency,
        category,
        target_days_per_week
    )
    .fetch_one(conn)
    .await?;

    Ok(row.into())
}

pub async fn find_by_id<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    owner: &AccountId,
) -> Result<Option<Habit>, sqlx::Error> {
    let row = sqlx::query_as!(
        HabitRow,
        r#"
        SELECT id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
        FROM habits
        WHERE id = $1 AND owner_id = $2
        "#,
        id,
        owner.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_by_name<'e>(
    conn: impl PgExecutor<'e>,
    name: &str,
    owner: &AccountId,
) -> Result<Option<Habit>, sqlx::Error> {
    let row = sqlx::query_as!(
        HabitRow,
        r#"
        SELECT id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
        FROM habits
        WHERE LOWER(name) = LOWER($1) AND owner_id = $2 AND archived = FALSE
        LIMIT 1
        "#,
        name,
        owner.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_all_by_owner<'e>(
    conn: impl PgExecutor<'e>,
    owner: &AccountId,
) -> Result<Vec<Habit>, sqlx::Error> {
    let rows = sqlx::query_as!(
        HabitRow,
        r#"
        SELECT id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
        FROM habits
        WHERE owner_id = $1 AND archived = FALSE
        ORDER BY id ASC
        "#,
        owner.as_uuid()
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn find_completed_log_dates<'e>(
    conn: impl PgExecutor<'e>,
    habit_id: i64,
    owner: &AccountId,
) -> Result<Vec<String>, sqlx::Error> {
    let dates = sqlx::query_scalar!(
        r#"
        SELECT log_date
        FROM habit_logs
        WHERE habit_id = $1 AND owner_id = $2 AND completed = TRUE
        ORDER BY log_date ASC
        "#,
        habit_id,
        owner.as_uuid()
    )
    .fetch_all(conn)
    .await?;

    Ok(dates)
}

pub async fn find_all_with_status(
    pool: &sqlx::PgPool,
    owner: &AccountId,
    today_str: &str,
) -> Result<Vec<HabitWithStatus>, sqlx::Error> {
    let habits = find_all_by_owner(pool, owner).await?;

    let today_date = NaiveDate::parse_from_str(today_str, "%Y-%m-%d")
        .unwrap_or_else(|_| chrono::Utc::now().naive_utc().date());

    let mut results = Vec::with_capacity(habits.len());

    for habit in habits {
        let log_rows = find_completed_log_dates(pool, habit.id, owner).await?;

        let date_set: HashSet<String> = log_rows.iter().cloned().collect();
        let (current_streak, longest_streak) = calculate_streaks(&log_rows, today_str);
        let completed_today = date_set.contains(today_str);
        let total_completions = log_rows.len() as i64;

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

pub async fn update_habit<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    owner: &AccountId,
    name: &str,
    description: Option<&str>,
    frequency: &str,
    category: &str,
    target_days_per_week: i64,
) -> Result<Option<Habit>, sqlx::Error> {
    let row = sqlx::query_as!(
        HabitRow,
        r#"
        UPDATE habits
        SET name = $1, description = $2, frequency = $3, category = $4, target_days_per_week = $5, updated_at = CAST(CURRENT_TIMESTAMP AS TEXT)
        WHERE id = $6 AND owner_id = $7
        RETURNING id, owner_id, name, description, frequency, category, target_days_per_week, archived, created_at, updated_at
        "#,
        name,
        description,
        frequency,
        category,
        target_days_per_week,
        id,
        owner.as_uuid()
    )
    .fetch_optional(conn)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn delete_habit<'e>(
    conn: impl PgExecutor<'e>,
    id: i64,
    owner: &AccountId,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "DELETE FROM habits WHERE id = $1 AND owner_id = $2",
        id,
        owner.as_uuid()
    )
    .execute(conn)
    .await?;

    Ok(res.rows_affected() > 0)
}

pub async fn toggle_log(
    pool: &sqlx::PgPool,
    habit_id: i64,
    owner: &AccountId,
    log_date: &str,
    note: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let existing = sqlx::query_scalar!(
        r#"
        SELECT completed
        FROM habit_logs
        WHERE habit_id = $1 AND owner_id = $2 AND log_date = $3
        "#,
        habit_id,
        owner.as_uuid(),
        log_date
    )
    .fetch_optional(pool)
    .await?;

    match existing {
        Some(completed) => {
            let new_state = !completed;
            sqlx::query!(
                r#"
                UPDATE habit_logs
                SET completed = $1
                WHERE habit_id = $2 AND owner_id = $3 AND log_date = $4
                "#,
                new_state,
                habit_id,
                owner.as_uuid(),
                log_date
            )
            .execute(pool)
            .await?;
            Ok(new_state)
        }
        None => {
            sqlx::query!(
                r#"
                INSERT INTO habit_logs (habit_id, owner_id, log_date, completed, note)
                VALUES ($1, $2, $3, TRUE, $4)
                "#,
                habit_id,
                owner.as_uuid(),
                log_date,
                note
            )
            .execute(pool)
            .await?;
            Ok(true)
        }
    }
}

pub async fn get_heatmap<'e>(
    conn: impl PgExecutor<'e>,
    owner: &AccountId,
) -> Result<Vec<HeatmapDay>, sqlx::Error> {
    let rows = sqlx::query!(
        r#"
        SELECT log_date AS "date!", COUNT(*) AS "count!"
        FROM habit_logs
        WHERE owner_id = $1 AND completed = TRUE
        GROUP BY log_date
        ORDER BY log_date ASC
        "#,
        owner.as_uuid()
    )
    .fetch_all(conn)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| HeatmapDay {
            date: r.date,
            count: r.count,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_habits_repository_crud() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let owner = AccountId::from_seed("user_habit_test");
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'test', 'active') ON CONFLICT DO NOTHING",
            owner.as_uuid()
        )
        .execute(&pool)
        .await
        .unwrap();

        let habit = create_habit(
            &pool,
            &owner,
            "Daily Reading",
            Some("Read 20 pages"),
            "daily",
            "education",
            7,
        )
        .await
        .unwrap();

        assert_eq!(habit.name, "Daily Reading");
        assert_eq!(habit.category, "education");

        let found = find_by_id(&pool, habit.id, &owner).await.unwrap().unwrap();
        assert_eq!(found.name, "Daily Reading");

        let found_name = find_by_name(&pool, "daily reading", &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found_name.id, habit.id);

        let toggled = toggle_log(&pool, habit.id, &owner, "2026-09-01", None)
            .await
            .unwrap();
        assert!(toggled);

        let toggled2 = toggle_log(&pool, habit.id, &owner, "2026-09-02", None)
            .await
            .unwrap();
        assert!(toggled2);

        let list_status = find_all_with_status(&pool, &owner, "2026-09-02")
            .await
            .unwrap();
        assert_eq!(list_status.len(), 1);
        assert!(list_status[0].completed_today);
        assert_eq!(list_status[0].current_streak, 2);

        let heatmap = get_heatmap(&pool, &owner).await.unwrap();
        assert_eq!(heatmap.len(), 2);

        let updated = update_habit(
            &pool,
            habit.id,
            &owner,
            "Night Reading",
            Some("Read 30 pages"),
            "daily",
            "learning",
            7,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.name, "Night Reading");

        let deleted = delete_habit(&pool, habit.id, &owner).await.unwrap();
        assert!(deleted);
    }
}
