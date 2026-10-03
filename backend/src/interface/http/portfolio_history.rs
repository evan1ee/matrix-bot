use axum::{
    extract::{Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortfolioHistoryPoint {
    pub date: String,
    pub holdings_value_minor: i64,
    pub cash_minor: i64,
    pub total_net_worth_minor: i64,
}

impl From<crate::domain::portfolio::history::PortfolioHistoryPoint> for PortfolioHistoryPoint {
    fn from(p: crate::domain::portfolio::history::PortfolioHistoryPoint) -> Self {
        Self {
            date: p.date,
            holdings_value_minor: p.holdings_value_minor,
            cash_minor: p.cash_minor,
            total_net_worth_minor: p.total_net_worth_minor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortfolioHistoryResponse {
    pub currency: String,
    pub currency_exponent: i64,
    pub timeframe: String,
    pub points: Vec<PortfolioHistoryPoint>,
    pub starting_value_minor: i64,
    pub current_value_minor: i64,
    pub period_change_minor: i64,
    pub period_change_pct: f64,
}

impl From<crate::domain::portfolio::history::PortfolioHistoryResponse>
    for PortfolioHistoryResponse
{
    fn from(r: crate::domain::portfolio::history::PortfolioHistoryResponse) -> Self {
        Self {
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            timeframe: r.timeframe,
            points: r
                .points
                .into_iter()
                .map(PortfolioHistoryPoint::from)
                .collect(),
            starting_value_minor: r.starting_value_minor,
            current_value_minor: r.current_value_minor,
            period_change_minor: r.period_change_minor,
            period_change_pct: r.period_change_pct,
        }
    }
}

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

    let history = state
        .services
        .portfolio
        .get_history(&user.account_id, timeframe, currency)
        .await?;

    Ok(Json(history.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn setup_state() -> (ApiState, AuthUser) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "http://localhost:8008".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser {
            account_id: crate::domain::accounts::AccountId::from_seed("user_test_hist"),
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
