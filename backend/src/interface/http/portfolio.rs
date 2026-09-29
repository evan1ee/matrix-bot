use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::domain::portfolio::models::{normalise_symbol, BuyOutcome, SellOutcome};
use crate::domain::portfolio::{PortfolioError, TradeRequest, MAX_NOTE_LEN};
use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

/// One investment position: symbol, quantity, and weighted-average cost basis.
///
/// `quantity_minor` and `avg_cost_minor` are always positive integers, scaled at
/// [`QUANTITY_EXPONENT`] and `currency_exponent` respectively. `current_price_minor`
/// is `None` until the user records one — an unset price is a normal state, not
/// an error, so market value and unrealized P&L are computed only when it is set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct Holding {
    pub id: i64,
    /// The sole ownership key, matching `AuthUser::owner_id()`.
    pub owner_id: String,
    pub symbol: String,
    pub quantity_minor: i64,
    pub avg_cost_minor: i64,
    pub currency: String,
    /// Decimal places for this row's currency, stored so a later change to the
    /// currency table cannot rewrite what this row means.
    pub currency_exponent: i64,
    pub current_price_minor: Option<i64>,
    pub price_updated_at: Option<DateTime<Utc>>,
    /// A single freeform comment, most-recent-wins — not a transaction log.
    pub note: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::portfolio::Holding> for Holding {
    fn from(h: crate::domain::portfolio::Holding) -> Self {
        Self {
            id: h.id,
            owner_id: h.owner_id.to_string(),
            symbol: h.symbol,
            quantity_minor: h.quantity_minor,
            avg_cost_minor: h.avg_cost_minor,
            currency: h.currency,
            currency_exponent: h.currency_exponent,
            current_price_minor: h.current_price_minor,
            price_updated_at: h.price_updated_at,
            note: h.note,
            created_at: h.created_at,
            updated_at: h.updated_at,
        }
    }
}

/// Cash available to spend on a buy, in one currency. A missing row means a
/// zero balance — the same "no row until you touch it" convention as
/// `settings::UserSettings`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct CashBalance {
    pub id: i64,
    /// The sole ownership key, matching `AuthUser::owner_id()`.
    pub owner_id: String,
    pub currency: String,
    pub currency_exponent: i64,
    /// Never negative — enforced in `PortfolioRepository`, not the schema, the
    /// same app-level-only convention `quantity_minor`/`avg_cost_minor` use.
    pub balance_minor: i64,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::portfolio::CashBalance> for CashBalance {
    fn from(c: crate::domain::portfolio::CashBalance) -> Self {
        Self {
            id: c.id,
            owner_id: c.owner_id.to_string(),
            currency: c.currency,
            currency_exponent: c.currency_exponent,
            balance_minor: c.balance_minor,
            updated_at: c.updated_at,
        }
    }
}

/// Totals for one currency. Currencies are never combined — without a rate
/// source, adding a USD position to a JPY position produces a meaningless
/// number, exactly as `finance::models::CurrencySummary` documents.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PortfolioCurrencySummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub cost_basis_minor: i64,
    /// Market value of only the holdings that have a price set.
    pub market_value_minor: i64,
    /// Unrealized P&L of only the holdings that have a price set.
    pub unrealized_pl_minor: i64,
    pub priced_holdings: i64,
    pub unpriced_holdings: i64,
    /// Present even for a currency with no holdings at all — a deposit with
    /// nothing bought yet must still show up here.
    pub cash_minor: i64,
}

impl From<crate::domain::portfolio::PortfolioCurrencySummary> for PortfolioCurrencySummary {
    fn from(s: crate::domain::portfolio::PortfolioCurrencySummary) -> Self {
        Self {
            currency: s.currency,
            currency_exponent: s.currency_exponent,
            cost_basis_minor: s.cost_basis_minor,
            market_value_minor: s.market_value_minor,
            unrealized_pl_minor: s.unrealized_pl_minor,
            priced_holdings: s.priced_holdings,
            unpriced_holdings: s.unpriced_holdings,
            cash_minor: s.cash_minor,
        }
    }
}

/// Amounts and quantities arrive as decimal strings, never as JSON numbers —
/// the same reason `finance::TransactionRequest` does: a JSON number is an
/// IEEE-754 double by the time most clients have parsed it.
#[derive(Deserialize, ToSchema)]
pub struct BuyHoldingRequest {
    /// Ticker/symbol, e.g. `"AAPL"`.
    pub symbol: String,
    /// Decimal string, e.g. `"10"` or `"0.5"`.
    pub quantity: String,
    /// Decimal string, e.g. `"150.00"`.
    pub price: String,
    /// ISO 4217 code, e.g. `"USD"`.
    pub currency: String,
    pub note: Option<String>,
    pub executed_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, ToSchema)]
pub struct SellHoldingRequest {
    pub quantity: String,
    pub price: String,
    pub executed_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, ToSchema)]
pub struct SetPriceRequest {
    pub price: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetNoteRequest {
    pub note: Option<String>,
}

pub use crate::interface::http::portfolio_cash::{prepare_cash_amount, CashAmountRequest};

#[derive(Debug, Serialize, ToSchema)]
pub struct SellResponse {
    /// `None` when the sale exactly closed the position.
    pub holding: Option<Holding>,
    /// May be negative.
    pub realized_pl_minor: i64,
}

impl From<PortfolioError> for ApiError {
    fn from(err: PortfolioError) -> Self {
        ApiError::bad_request(err.to_string())
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/holdings",
    responses(
        (status = 200, description = "Your holdings", body = Vec<Holding>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_holdings(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<Holding>>, ApiError> {
    let holdings = state
        .services
        .portfolio
        .list_holdings(&user.account_id)
        .await?;

    Ok(Json(holdings.into_iter().map(Holding::from).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/holdings",
    request_body = BuyHoldingRequest,
    responses(
        (status = 200, description = "Position opened or added to", body = Holding),
        (status = 400, description = "Bad request, or insufficient cash", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn buy_holding(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<BuyHoldingRequest>,
) -> Result<Json<Holding>, ApiError> {
    let symbol = normalise_symbol(&payload.symbol).map_err(ApiError::bad_request)?;
    let currency =
        normalise_currency(&payload.currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);

    let outcome = state
        .services
        .portfolio
        .buy(
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
        .await?;

    match outcome {
        BuyOutcome::Ok { holding, .. } => Ok(Json((*holding).into())),
        BuyOutcome::CurrencyMismatch { existing_currency } => Err(ApiError::bad_request(format!(
            "{} is already held in {}",
            symbol, existing_currency
        ))),
        BuyOutcome::InsufficientCash {
            available_minor,
            needed_minor,
        } => Err(ApiError::bad_request(format!(
            "Need {} {} but only {} {} in cash",
            format_for_error(needed_minor, exponent),
            currency,
            format_for_error(available_minor, exponent),
            currency
        ))),
    }
}

pub(crate) fn format_for_error(minor: i64, exponent: u32) -> String {
    crate::domain::finance::money::format_amount(minor, exponent)
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/holdings/{id}/sell",
    params(("id" = i64, Path, description = "Holding ID")),
    request_body = SellHoldingRequest,
    responses(
        (status = 200, description = "Position reduced or closed", body = SellResponse),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn sell_holding(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<SellHoldingRequest>,
) -> Result<Json<SellResponse>, ApiError> {
    let existing = state
        .services
        .portfolio
        .get_holding_owned(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    let outcome = state
        .services
        .portfolio
        .sell(
            &user.account_id,
            &existing.symbol,
            &payload.quantity,
            &payload.price,
            payload.executed_at,
        )
        .await?;

    match outcome {
        SellOutcome::NotFound => Err(ApiError::not_found("Holding not found or access denied")),
        SellOutcome::ExceedsHolding { held_minor } => Err(ApiError::bad_request(format!(
            "Only {} held (minor units)",
            held_minor
        ))),
        SellOutcome::Closed {
            realized_pl_minor, ..
        } => Ok(Json(SellResponse {
            holding: None,
            realized_pl_minor,
        })),
        SellOutcome::Reduced {
            holding,
            realized_pl_minor,
            ..
        } => Ok(Json(SellResponse {
            holding: Some(holding.into()),
            realized_pl_minor,
        })),
    }
}

#[utoipa::path(
    put,
    path = "/api/v1/portfolio/holdings/{id}/price",
    params(("id" = i64, Path, description = "Holding ID")),
    request_body = SetPriceRequest,
    responses(
        (status = 200, description = "Price updated", body = Holding),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn set_price(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<SetPriceRequest>,
) -> Result<Json<Holding>, ApiError> {
    let existing = state
        .services
        .portfolio
        .get_holding_owned(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    let exponent = existing.currency_exponent as u32;
    let price_minor =
        parse_amount(&payload.price, exponent).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let holding = state
        .services
        .portfolio
        .set_price(&user.account_id, &existing.symbol, price_minor)
        .await?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    Ok(Json(holding.into()))
}

#[utoipa::path(
    put,
    path = "/api/v1/portfolio/holdings/{id}/note",
    params(("id" = i64, Path, description = "Holding ID")),
    request_body = SetNoteRequest,
    responses(
        (status = 200, description = "Note updated", body = Holding),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn set_note(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<SetNoteRequest>,
) -> Result<Json<Holding>, ApiError> {
    let note = match &payload.note {
        Some(text) if text.len() > MAX_NOTE_LEN => {
            return Err(ApiError::bad_request(format!(
                "Note exceeds {} character limit",
                MAX_NOTE_LEN
            )))
        }
        Some(text) if text.trim().is_empty() => None,
        Some(text) => Some(text.as_str()),
        None => None,
    };

    let existing = state
        .services
        .portfolio
        .get_holding_owned(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    let holding = state
        .services
        .portfolio
        .set_note(&user.account_id, &existing.symbol, note)
        .await?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    Ok(Json(holding.into()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/portfolio/holdings/{id}",
    params(("id" = i64, Path, description = "Holding ID")),
    responses(
        (status = 204, description = "Holding removed"),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_holding(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let existing = state
        .services
        .portfolio
        .get_holding_owned(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    state
        .services
        .portfolio
        .delete_holding(&user.account_id, &existing.symbol)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/summary",
    operation_id = "portfolio_summary",
    responses(
        (status = 200, description = "Per-currency totals", body = Vec<PortfolioCurrencySummary>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn summary(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<PortfolioCurrencySummary>>, ApiError> {
    let summaries = state.services.portfolio.summary(&user.account_id).await?;

    Ok(Json(
        summaries
            .into_iter()
            .map(PortfolioCurrencySummary::from)
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use crate::interface::http::portfolio_cash::deposit_cash;

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

    async fn fund(state: &ApiState, user: &AuthUser, currency: &str) {
        let _ = deposit_cash(
            State(state.clone()),
            user.clone(),
            Json(CashAmountRequest {
                amount: "1000000".to_string(),
                currency: currency.to_string(),
                executed_at: None,
            }),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn test_buy_sell_flow() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        fund(&state, &alice, "USD").await;

        let buy_req = BuyHoldingRequest {
            symbol: "AAPL".to_string(),
            quantity: "10".to_string(),
            price: "150.00".to_string(),
            currency: "USD".to_string(),
            note: Some("first buy".to_string()),
            executed_at: None,
        };

        let bought = buy_holding(State(state.clone()), alice.clone(), Json(buy_req))
            .await
            .unwrap()
            .0;
        assert_eq!(bought.symbol, "AAPL");
        assert_eq!(bought.quantity_minor, 10 * 10i64.pow(8));

        let sell_req = SellHoldingRequest {
            quantity: "5".to_string(),
            price: "180.00".to_string(),
            executed_at: None,
        };

        let sold = sell_holding(
            State(state.clone()),
            alice.clone(),
            Path(bought.id),
            Json(sell_req),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(sold.realized_pl_minor, 15_000);
        assert!(sold.holding.is_some());
    }

    #[tokio::test]
    async fn test_summary_reflects_positions() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        fund(&state, &alice, "USD").await;

        let res = summary(State(state.clone()), alice.clone())
            .await
            .unwrap()
            .0;
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].currency, "USD");
    }
}
