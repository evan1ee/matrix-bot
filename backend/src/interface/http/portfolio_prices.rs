use axum::{
    extract::{Path, State},
    Json,
};

use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::modules::portfolio::audit::AuditVerificationResult;
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
    let summary = refresh_all_for_owner(&state.db.pool, Some(&state.broadcaster), &owner_id).await;

    let _ =
        crate::modules::portfolio::history::HistoryRepository::capture_current_snapshot_for_owner(
            &state.db.pool,
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
    Json(payload): Json<crate::interface::http::portfolio::BuyHoldingRequest>,
) -> Result<Json<Holding>, ApiError> {
    use crate::modules::portfolio::service::{self, TradeRequest};

    let owner_id = user.owner_id();

    let result = service::record(
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
        TradeRequest {
            symbol: &payload.symbol,
            quantity: &payload.quantity,
            price: &payload.price,
            currency: &payload.currency,
            note: payload.note.as_deref(),
            executed_at: payload.executed_at,
        },
    )
    .await?;

    match result {
        Ok(holding) => Ok(Json(holding)),
        Err(err) => Err(ApiError::bad_request(err)),
    }
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct ActivityQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/activity",
    params(ActivityQuery),
    responses(
        (status = 200, description = "Recent portfolio operations audit log", body = Vec<PortfolioAuditEntry>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_portfolio_activity(
    State(state): State<ApiState>,
    user: AuthUser,
    axum::extract::Query(params): axum::extract::Query<ActivityQuery>,
) -> Result<Json<Vec<crate::modules::portfolio::audit::PortfolioAuditEntry>>, ApiError> {
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let entries = crate::modules::portfolio::audit::list_audit_entries(
        &state.db.pool,
        &user.owner_id(),
        limit,
        offset,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(entries))
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct SearchQuery {
    pub q: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/search",
    params(SearchQuery),
    responses(
        (status = 200, description = "Ticker symbol search results", body = Vec<TickerSearchResult>)
    )
)]
pub async fn search_ticker_symbols(
    State(_state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<Json<Vec<crate::modules::portfolio::price_feed::TickerSearchResult>>, ApiError> {
    let results = crate::modules::portfolio::price_feed::search_symbols(&params.q)
        .await
        .unwrap_or_default();

    Ok(Json(results))
}

#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct QuoteQuery {
    pub symbol: String,
}

#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
pub struct SymbolQuoteResponse {
    pub symbol: String,
    pub price: f64,
    pub currency: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/quote",
    params(QuoteQuery),
    responses(
        (status = 200, description = "Current market quote for symbol", body = SymbolQuoteResponse),
        (status = 400, description = "Failed to fetch quote", body = ApiErrorResponse)
    )
)]
pub async fn get_symbol_quote(
    axum::extract::Query(params): axum::extract::Query<QuoteQuery>,
) -> Result<Json<SymbolQuoteResponse>, ApiError> {
    let fetched = crate::modules::portfolio::price_feed::fetch_latest_price(&params.symbol)
        .await
        .map_err(ApiError::bad_request)?;

    Ok(Json(SymbolQuoteResponse {
        symbol: params.symbol.to_uppercase(),
        price: fetched.price,
        currency: fetched.currency,
    }))
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateSymbolRequest {
    pub symbol: String,
}

#[utoipa::path(
    put,
    path = "/api/v1/portfolio/holdings/{id}/symbol",
    request_body = UpdateSymbolRequest,
    responses(
        (status = 200, description = "Holding symbol updated", body = Holding),
        (status = 400, description = "Invalid symbol or symbol already exists", body = ApiErrorResponse),
        (status = 404, description = "Holding not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_holding_symbol(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateSymbolRequest>,
) -> Result<Json<Holding>, ApiError> {
    let owner_id = user.owner_id();

    match PortfolioRepository::update_symbol(&state.db.pool, &owner_id, id, &payload.symbol)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?
    {
        Ok(holding) => {
            state
                .broadcaster
                .send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
            Ok(Json(holding))
        }
        Err(err) => Err(ApiError::bad_request(err)),
    }
}

/// Verifies the cryptographic SHA-256 hash chain of the user's audit log.
#[utoipa::path(
    get,
    path = "/api/v1/portfolio/activity/verify",
    tag = "portfolio",
    responses(
        (status = 200, description = "Cryptographic verification report", body = AuditVerificationResult),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn verify_portfolio_activity_chain(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<AuditVerificationResult>, ApiError> {
    let owner_id = user.owner_id();
    let report = crate::modules::portfolio::audit::verify_audit_chain(&state.db.pool, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        ApiState::new(
            crate::db::Database { pool: pool },
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

    #[tokio::test]
    async fn test_update_holding_symbol_api() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let Json(recorded) = record_holding_position(
            State(state.clone()),
            alice.clone(),
            Json(crate::interface::http::portfolio::BuyHoldingRequest {
                symbol: "AAPLL".to_string(),
                quantity: "10".to_string(),
                price: "150.00".to_string(),
                currency: "USD".to_string(),
                note: None,
                executed_at: None,
            }),
        )
        .await
        .unwrap();

        assert_eq!(recorded.symbol, "AAPLL");

        let Json(updated) = update_holding_symbol(
            State(state.clone()),
            alice.clone(),
            Path(recorded.id),
            Json(UpdateSymbolRequest {
                symbol: "AAPL".to_string(),
            }),
        )
        .await
        .unwrap();

        assert_eq!(updated.symbol, "AAPL");
    }
}
