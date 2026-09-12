use axum::{
    extract::{Path, State},
    Json,
};
use chrono::NaiveDate;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::api::auth::{ApiState, AuthUser};
use crate::api::life_activity_collectors::*;
use crate::error::ApiError;

/// A single event from any module that happened on a given date.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityEvent {
    /// ISO time within the day, e.g. "09:15:00" (or null if date-only)
    pub time: Option<String>,
    /// Source module identifier
    pub module: &'static str,
    /// Verb: "created", "completed", "expense", "income", "buy", "sell", etc.
    pub action: String,
    /// Visual emoji for the event type
    pub emoji: &'static str,
    /// One-line human summary
    pub summary: String,
    /// Optional secondary detail
    pub detail: Option<String>,
}

/// Summary counts per module.
#[derive(Debug, Clone, Serialize, Default)]
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

#[derive(Debug, Clone, Serialize)]
pub struct DayActivityResponse {
    pub date: String,
    pub events: Vec<ActivityEvent>,
    pub stats: ActivityStats,
}

pub async fn collect_day_activity(
    pool: &SqlitePool,
    owner: &str,
    date: &str,
) -> DayActivityResponse {
    let mut events: Vec<ActivityEvent> = Vec::new();
    let mut stats = ActivityStats::default();

    // --- Todos ---
    query_todos(pool, owner, date, &mut events, &mut stats).await;

    // --- Finance Transactions ---
    query_finance(pool, owner, date, &mut events, &mut stats).await;

    // --- Portfolio Audit Log ---
    query_portfolio(pool, owner, date, &mut events, &mut stats).await;

    // --- Habits ---
    query_habits(pool, owner, date, &mut events, &mut stats).await;

    // --- Reminders ---
    query_reminders(pool, owner, date, &mut events, &mut stats).await;

    // --- Bookmarks ---
    query_bookmarks(pool, owner, date, &mut events, &mut stats).await;

    // --- Footprints ---
    query_footprints(pool, owner, date, &mut events, &mut stats).await;

    // Sort chronologically (nulls last)
    events.sort_by(|a, b| a.time.cmp(&b.time));
    stats.total_events = events.len();

    DayActivityResponse {
        date: date.to_string(),
        events,
        stats,
    }
}

pub async fn get_day_activity(
    user: AuthUser,
    State(state): State<ApiState>,
    Path(date): Path<String>,
) -> Result<Json<DayActivityResponse>, ApiError> {
    if NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() {
        return Err(ApiError::bad_request(
            "Invalid date format. Use YYYY-MM-DD.",
        ));
    }

    let response = collect_day_activity(&state.db.pool, &user.account_id, &date).await;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::db::Database;
    use crate::events::EventBroadcaster;

    async fn create_test_state() -> (ApiState, SqlitePool) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let db = Database { pool: pool.clone() };
        let broadcaster = EventBroadcaster::new(10);
        let client = reqwest::Client::new();
        let state = ApiState::new(
            db,
            broadcaster,
            Some("secret".to_string()),
            "https://matrix.org".to_string(),
            client,
        );
        (state, pool)
    }

    fn test_user(account_id: &str) -> AuthUser {
        AuthUser {
            account_id: account_id.to_string(),
            display_name: account_id.to_string(),
            session_id_hash: "sess_hash".to_string(),
            last_verified_at: chrono::Utc::now(),
            restricted_until: None,
        }
    }

    #[tokio::test]
    async fn test_empty_day_returns_zero_events() {
        let (state, _pool) = create_test_state().await;
        let user = test_user("test-user");
        let result = get_day_activity(
            user,
            axum::extract::State(state),
            Path("2026-09-06".to_string()),
        )
        .await
        .unwrap();

        let body = result.0;
        assert_eq!(body.date, "2026-09-06");
        assert_eq!(body.events.len(), 0);
        assert_eq!(body.stats.total_events, 0);
    }

    #[tokio::test]
    async fn test_todo_created_on_date_appears_in_feed() {
        let (state, pool) = create_test_state().await;

        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed, created_at) \
             VALUES ('!r', 'user1', 'Alice', 'Buy groceries', 'high', 0, '2026-09-06 09:15:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let user = test_user("user1");
        let result = get_day_activity(
            user,
            axum::extract::State(state),
            Path("2026-09-06".to_string()),
        )
        .await
        .unwrap();

        let body = result.0;
        assert_eq!(body.stats.todos_created, 1);
        assert_eq!(body.events.len(), 1);
        assert_eq!(body.events[0].module, "todo");
        assert_eq!(body.events[0].action, "created");
        assert!(body.events[0].summary.contains("Buy groceries"));
    }

    #[tokio::test]
    async fn test_finance_transaction_appears_in_feed() {
        let (state, pool) = create_test_state().await;

        sqlx::query(
            "INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, \
             currency, currency_exponent, occurred_at) \
             VALUES ('!r', 'user1', 'Alice', 'expense', 4250, 'USD', 2, '2026-09-06 10:30:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let user = test_user("user1");
        let result = get_day_activity(
            user,
            axum::extract::State(state),
            Path("2026-09-06".to_string()),
        )
        .await
        .unwrap();

        let body = result.0;
        assert_eq!(body.stats.expenses, 1);
        assert!(body.events[0].summary.contains("42.50"));
    }

    #[tokio::test]
    async fn test_invalid_date_rejected() {
        let (state, _pool) = create_test_state().await;
        let user = test_user("user1");
        let result = get_day_activity(
            user,
            axum::extract::State(state),
            Path("not-a-date".to_string()),
        )
        .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_footprints_only_appear_on_arrival_or_departure_date() {
        let (state, pool) = create_test_state().await;

        // Trip from 2026-09-04 to 2026-09-14
        sqlx::query(
            "INSERT INTO footprints (owner_id, title, latitude, longitude, city, arrival_date, departure_date, created_at) \
             VALUES ('user1', 'Danxia Mountain', 25.0, 113.0, 'Shaoguan', '2026-09-04', '2026-09-14', '2026-09-04 01:48:57')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let user = test_user("user1");

        // 1. On arrival date: appears as "arrived"
        let res_arrival = get_day_activity(
            user.clone(),
            axum::extract::State(state.clone()),
            Path("2026-09-04".to_string()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_arrival.stats.footprints, 1);
        assert_eq!(res_arrival.events[0].action, "arrived");

        // 2. On intermediate date (e.g. 2026-09-10 or 2026-09-11): does NOT appear!
        let res_mid = get_day_activity(
            user.clone(),
            axum::extract::State(state.clone()),
            Path("2026-09-10".to_string()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_mid.stats.footprints, 0);
        assert_eq!(res_mid.events.len(), 0);

        // 3. On departure date: appears as "departed"
        let res_dep = get_day_activity(
            user,
            axum::extract::State(state),
            Path("2026-09-14".to_string()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_dep.stats.footprints, 1);
        assert_eq!(res_dep.events[0].action, "departed");
    }
}
