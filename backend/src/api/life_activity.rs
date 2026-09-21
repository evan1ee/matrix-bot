use axum::{
    extract::{Path, State},
    Json,
};
use chrono::NaiveDate;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::life::activity::{collect_day_activity, DayActivityResponse};

#[utoipa::path(
    get,
    tag = "life",
    path = "/api/v1/life/days/{date}/activity",
    params(("date" = String, Path, description = "Day, as YYYY-MM-DD")),
    responses(
        (status = 200, description = "Everything that happened across every module on that day", body = DayActivityResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
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

    let response = collect_day_activity(state.db.sqlite_pool(), &user.account_id, &date).await;
    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::db::Database;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    async fn create_test_state() -> (ApiState, SqlitePool) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&crate::db::DbPool::Sqlite(pool.clone()))
            .await
            .unwrap();
        let db = Database {
            pool: crate::db::DbPool::Sqlite(pool.clone()),
        };
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
    async fn test_footprints_only_appear_on_arrival_date() {
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

        // 2. On intermediate date: does NOT appear
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

        // 3. On departure date: does NOT appear
        let res_dep = get_day_activity(
            user,
            axum::extract::State(state),
            Path("2026-09-14".to_string()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(res_dep.stats.footprints, 0);
        assert_eq!(res_dep.events.len(), 0);
    }
}
