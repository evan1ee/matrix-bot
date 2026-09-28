use axum::{
    extract::{Query, State},
    Json,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::modules::portfolio::history::{HistoryRepository, PortfolioHistoryResponse};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct HistoryQuery {
    /// Timeframe: "7d", "1m", "3m", "1y", "all". Defaults to "1m".
    pub timeframe: Option<String>,
    /// Currency code, e.g. "USD". Defaults to "USD".
    pub currency: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/history",
    params(HistoryQuery),
    responses(
        (status = 200, description = "Historical net worth data points", body = PortfolioHistoryResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_portfolio_history(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<HistoryQuery>,
) -> Result<Json<PortfolioHistoryResponse>, ApiError> {
    let timeframe = query.timeframe.as_deref().unwrap_or("1m");
    let currency = query.currency.as_deref().unwrap_or("USD");

    let history =
        HistoryRepository::get_history(&state.db.pool, &user.owner_id(), timeframe, currency)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(history))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn setup_state() -> (ApiState, AuthUser) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let broadcaster = EventBroadcaster::default();
        let state = ApiState::new(
            crate::db::Database { pool: pool },
            broadcaster,
            None,
            "http://localhost:8008".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser {
            account_id: crate::domain::accounts::AccountId::new("user_test_hist"),
            display_name: "Test Hist User".to_string(),
            session_id_hash: "hash".to_string(),
            last_verified_at: chrono::Utc::now(),
            restricted_until: None,
        };
        (state, user)
    }

    #[tokio::test]
    async fn test_get_portfolio_history_api() {
        let (state, user) = setup_state().await;

        let query = HistoryQuery {
            timeframe: Some("7d".to_string()),
            currency: Some("USD".to_string()),
        };

        let Json(res) = get_portfolio_history(State(state), user, Query(query))
            .await
            .unwrap();

        assert_eq!(res.currency, "USD");
        assert_eq!(res.timeframe, "7d");
        assert!(!res.points.is_empty());
    }
}
