use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::portfolio::TradeRequest;
use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::portfolio::{BuyHoldingRequest, Holding};

#[derive(Debug, Default, Clone, Serialize, ToSchema)]
pub struct RefreshSummary {
    pub updated: Vec<String>,
    pub failed: Vec<String>,
}

impl From<crate::application::portfolio::RefreshSummary> for RefreshSummary {
    fn from(s: crate::application::portfolio::RefreshSummary) -> Self {
        Self {
            updated: s.updated,
            failed: s.failed,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TickerSearchResult {
    pub symbol: String,
    pub name: String,
    pub exchange: Option<String>,
    pub quote_type: Option<String>,
}

impl From<crate::infrastructure::web::portfolio_feed::TickerSearchResult> for TickerSearchResult {
    fn from(r: crate::infrastructure::web::portfolio_feed::TickerSearchResult) -> Self {
        Self {
            symbol: r.symbol,
            name: r.name,
            exchange: r.exchange,
            quote_type: r.quote_type,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SymbolQuoteResponse {
    pub symbol: String,
    pub price: f64,
    pub currency: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateSymbolRequest {
    pub symbol: String,
}

/// An immutable, cryptographically signed record of a portfolio activity.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PortfolioAuditEntry {
    pub id: i64,
    pub owner_id: String,
    /// Operation kind: "buy", "sell", "deposit", "withdraw", "record"
    pub action: String,
    pub symbol: Option<String>,
    pub quantity_minor: Option<i64>,
    pub price_minor: Option<i64>,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub realized_pl_minor: Option<i64>,
    pub note: Option<String>,
    pub prev_hash: String,
    pub entry_hash: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::portfolio::audit::PortfolioAuditEntry> for PortfolioAuditEntry {
    fn from(e: crate::domain::portfolio::audit::PortfolioAuditEntry) -> Self {
        Self {
            id: e.id,
            owner_id: e.owner_id.to_string(),
            action: e.action,
            symbol: e.symbol,
            quantity_minor: e.quantity_minor,
            price_minor: e.price_minor,
            amount_minor: e.amount_minor,
            currency: e.currency,
            currency_exponent: e.currency_exponent,
            realized_pl_minor: e.realized_pl_minor,
            note: e.note,
            prev_hash: e.prev_hash,
            entry_hash: e.entry_hash,
            created_at: e.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct AuditVerificationResult {
    pub is_valid: bool,
    pub total_verified: usize,
    pub broken_entry_id: Option<i64>,
    pub head_hash: Option<String>,
    pub genesis_hash: String,
    pub message: String,
}

impl From<crate::domain::portfolio::audit::AuditVerificationResult> for AuditVerificationResult {
    fn from(r: crate::domain::portfolio::audit::AuditVerificationResult) -> Self {
        Self {
            is_valid: r.is_valid,
            total_verified: r.total_verified,
            broken_entry_id: r.broken_entry_id,
            head_hash: r.head_hash,
            genesis_hash: r.genesis_hash,
            message: r.message,
        }
    }
}

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
    let summary = state
        .services
        .portfolio
        .refresh_all_for_owner(&user.account_id)
        .await;

    let _ = state
        .services
        .portfolio
        .capture_current_snapshot_for_owner(&user.account_id)
        .await;

    Ok(Json(summary.into()))
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
    let holding = state
        .services
        .portfolio
        .get_holding_owned(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Holding with id {} not found", id)))?;

    match state
        .services
        .portfolio
        .refresh_holding_price(&user.account_id, &holding)
        .await
    {
        Ok(updated) => Ok(Json(updated.into())),
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
    Json(payload): Json<BuyHoldingRequest>,
) -> Result<Json<Holding>, ApiError> {
    match state
        .services
        .portfolio
        .record(
            &user.account_id,
            TradeRequest {
                symbol: &payload.symbol,
                quantity: &payload.quantity,
                price: &payload.price,
                currency: &payload.currency,
                note: payload.note.as_deref(),
                executed_at: payload.executed_at,
            },
        )
        .await?
    {
        Ok(holding) => Ok(Json(holding.into())),
        Err(err) => Err(ApiError::bad_request(err)),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
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
) -> Result<Json<Vec<PortfolioAuditEntry>>, ApiError> {
    let limit = params.limit.unwrap_or(50);
    let offset = params.offset.unwrap_or(0);

    let entries = state
        .services
        .portfolio
        .list_audit_entries(&user.account_id, limit, offset)
        .await?;

    Ok(Json(
        entries.into_iter().map(PortfolioAuditEntry::from).collect(),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
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
    State(state): State<ApiState>,
    axum::extract::Query(params): axum::extract::Query<SearchQuery>,
) -> Result<Json<Vec<TickerSearchResult>>, ApiError> {
    let results = state
        .services
        .portfolio
        .search_symbols(&params.q)
        .await
        .unwrap_or_default();

    Ok(Json(
        results.into_iter().map(TickerSearchResult::from).collect(),
    ))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct QuoteQuery {
    pub symbol: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SymbolQuoteResponseWrapper {
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
    let fetched = crate::infrastructure::web::portfolio_feed::fetch_latest_price(&params.symbol)
        .await
        .map_err(ApiError::bad_request)?;

    Ok(Json(SymbolQuoteResponse {
        symbol: params.symbol.to_uppercase(),
        price: fetched.price,
        currency: fetched.currency,
    }))
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
    match state
        .services
        .portfolio
        .update_symbol(&user.account_id, id, &payload.symbol)
        .await?
    {
        Ok(holding) => Ok(Json(holding.into())),
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
    let report = state
        .services
        .portfolio
        .verify_audit_chain(&user.account_id)
        .await?;

    Ok(Json(report.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice"]).await;
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

    #[tokio::test]
    async fn test_update_holding_symbol_api() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let Json(recorded) = record_holding_position(
            State(state.clone()),
            alice.clone(),
            Json(BuyHoldingRequest {
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
