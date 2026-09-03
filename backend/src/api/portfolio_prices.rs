use axum::{
    extract::{Path, State},
    Json,
};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::portfolio::models::Holding;
use crate::modules::portfolio::price_feed::{
    refresh_all_for_owner, refresh_holding_price, RefreshSummary,
};
use crate::modules::portfolio::repository::PortfolioRepository;

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/refresh-prices",
    responses(
        (status = 200, description = "Refreshed prices for all holdings", body = RefreshSummary),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn refresh_all_prices(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<RefreshSummary>, ApiError> {
    let owner_id = user.owner_id();
    let summary = refresh_all_for_owner(
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
    )
    .await;

    Ok(Json(summary))
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/holdings/{id}/refresh-price",
    params(
        ("id" = i64, Path, description = "Holding database ID")
    ),
    responses(
        (status = 200, description = "Holding price refreshed", body = Holding),
        (status = 404, description = "Holding not found", body = ApiErrorResponse),
        (status = 502, description = "Failed to fetch market price", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn refresh_single_holding_price(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<Holding>, ApiError> {
    let owner_id = user.owner_id();
    let holding = PortfolioRepository::find_owned(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found(format!("Holding with id {} not found", id)))?;

    match refresh_holding_price(&state.db.pool, Some(&state.broadcaster), &holding).await {
        Ok(updated) => Ok(Json(updated)),
        Err(err) => Err(ApiError::bad_gateway(format!(
            "Failed to fetch market price for {}: {}",
            holding.symbol, err
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    async fn test_state() -> ApiState {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_refresh_all_prices_empty() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let Json(summary) = refresh_all_prices(State(state), alice).await.unwrap();
        assert!(summary.updated.is_empty());
        assert!(summary.failed.is_empty());
    }

    #[tokio::test]
    async fn test_refresh_unknown_holding_returns_404() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let err = refresh_single_holding_price(State(state), alice, Path(999))
            .await
            .unwrap_err();
        assert_eq!(err.code(), 404);
    }
}
