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
        Err(err) => Err(ApiError::bad_request(format!(
            "Failed to fetch market price for {}: {}",
            holding.symbol, err
        ))),
    }
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/holdings/record",
    request_body = BuyHoldingRequest,
    responses(
        (status = 200, description = "Position recorded without cash debit", body = Holding),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn record_holding_position(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<crate::api::portfolio::BuyHoldingRequest>,
) -> Result<Json<Holding>, ApiError> {
    use crate::events::SystemEvent;
    use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
    use crate::modules::portfolio::models::{normalise_symbol, QUANTITY_EXPONENT};

    let owner_id = user.owner_id();
    let symbol = normalise_symbol(&payload.symbol)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let currency = normalise_currency(&payload.currency)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);

    let quantity_minor = parse_amount(&payload.quantity, QUANTITY_EXPONENT)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let price_minor = parse_amount(&payload.price, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let note = match payload.note {
        Some(ref n) if n.trim().is_empty() => None,
        Some(ref n) => Some(n.as_str()),
        None => None,
    };

    match PortfolioRepository::record_holding(
        &state.db.pool,
        &owner_id,
        &symbol,
        quantity_minor,
        price_minor,
        &currency,
        exponent as i64,
        note,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    {
        Ok(holding) => {
            state.broadcaster.send(SystemEvent::PortfolioHoldingCreated(holding.clone()));
            Ok(Json(holding))
        }
        Err(err) => Err(ApiError::bad_request(err)),
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
        assert_eq!(err.status, axum::http::StatusCode::NOT_FOUND);
    }
}
