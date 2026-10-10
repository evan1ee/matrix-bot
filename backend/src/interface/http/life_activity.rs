use axum::{
    extract::{Path, State},
    Json,
};
use chrono::NaiveDate;
use serde::Serialize;
use utoipa::ToSchema;

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

/// A single event from any module that happened on a given date.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ActivityEvent {
    /// ISO time within the day, e.g. "09:15:00" (or null if date-only)
    pub time: Option<String>,
    /// Source module identifier
    pub module: String,
    /// Verb: "created", "completed", "expense", "income", "buy", "sell", etc.
    pub action: String,
    /// Visual emoji for the event type
    pub emoji: String,
    /// One-line human summary
    pub summary: String,
    /// Optional secondary detail
    pub detail: Option<String>,
}

impl From<crate::domain::life::ActivityEvent> for ActivityEvent {
    fn from(e: crate::domain::life::ActivityEvent) -> Self {
        Self {
            time: e.time,
            module: e.module,
            action: e.action,
            emoji: e.emoji,
            summary: e.summary,
            detail: e.detail,
        }
    }
}

/// Summary counts per module.
#[derive(Debug, Clone, Serialize, Default, ToSchema)]
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

impl From<crate::domain::life::ActivityStats> for ActivityStats {
    fn from(s: crate::domain::life::ActivityStats) -> Self {
        Self {
            total_events: s.total_events,
            todos_created: s.todos_created,
            todos_completed: s.todos_completed,
            todos_due: s.todos_due,
            expenses: s.expenses,
            income: s.income,
            portfolio_trades: s.portfolio_trades,
            habits_done: s.habits_done,
            reminders_fired: s.reminders_fired,
            bookmarks_saved: s.bookmarks_saved,
            bills_paid: s.bills_paid,
            footprints: s.footprints,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct DayActivityResponse {
    pub date: String,
    pub events: Vec<ActivityEvent>,
    pub stats: ActivityStats,
}

impl From<crate::domain::life::DayActivityResponse> for DayActivityResponse {
    fn from(r: crate::domain::life::DayActivityResponse) -> Self {
        Self {
            date: r.date,
            events: r.events.into_iter().map(ActivityEvent::from).collect(),
            stats: ActivityStats::from(r.stats),
        }
    }
}

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

    let response = state
        .services
        .life
        .collect_day_activity(&user.account_id, &date)
        .await?;
    Ok(Json(DayActivityResponse::from(response)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db::DbPool;

    async fn create_test_state() -> (ApiState, DbPool) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let db = Database { pool: pool.clone() };
        let client = reqwest::Client::new();
        let state = ApiState::new(
            db,
            Some("secret".to_string()),
            "https://matrix.org".to_string(),
            client,
        );
        (state, pool)
    }

    fn test_user(account_id: &str) -> AuthUser {
        AuthUser {
            account_id: crate::domain::accounts::AccountId::from_seed(account_id),
            display_name: account_id.to_string(),
            session_id_hash: "sess_hash".to_string(),
            last_verified_at: crate::utils::time::now_at_storage_precision(),
            restricted_until: None,
            role: crate::domain::accounts::AccountRole::User,
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
}
