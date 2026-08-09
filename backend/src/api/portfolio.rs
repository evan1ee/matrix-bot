use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::portfolio::models::{
    normalise_symbol, CashBalance, Holding, PortfolioCurrencySummary, QUANTITY_EXPONENT,
};
use crate::modules::portfolio::repository::{BuyOutcome, PortfolioRepository, SellOutcome, WithdrawOutcome};

const MAX_NOTE_LEN: usize = 500;

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
}

#[derive(Deserialize, ToSchema)]
pub struct SellHoldingRequest {
    pub quantity: String,
    pub price: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetPriceRequest {
    pub price: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetNoteRequest {
    pub note: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CashAmountRequest {
    /// Decimal string, e.g. `"1000.00"`.
    pub amount: String,
    /// ISO 4217 code, e.g. `"USD"`.
    pub currency: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SellResponse {
    /// `None` when the sale exactly closed the position.
    pub holding: Option<Holding>,
    /// May be negative.
    pub realized_pl_minor: i64,
}

fn note_or_reject(note: &Option<String>) -> Result<Option<&str>, ApiError> {
    match note {
        Some(text) if text.len() > MAX_NOTE_LEN => Err(ApiError::bad_request(format!(
            "Note exceeds {} character limit",
            MAX_NOTE_LEN
        ))),
        Some(text) if text.trim().is_empty() => Ok(None),
        Some(text) => Ok(Some(text.as_str())),
        None => Ok(None),
    }
}

/// Validates a `{amount, currency}` cash request, returning minor units.
fn prepare_cash_amount(payload: &CashAmountRequest) -> Result<(i64, String, i64), ApiError> {
    let currency = normalise_currency(&payload.currency)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(&payload.amount, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok((amount_minor, currency, exponent as i64))
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/holdings",
    responses(
        (status = 200, description = "Your holdings", body = Vec<Holding>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_holdings(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<Holding>>, ApiError> {
    let holdings = PortfolioRepository::list(&state.db.pool, &user.owner_id())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(holdings))
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
    security(("bearer_auth" = []))
)]
pub async fn buy_holding(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<BuyHoldingRequest>,
) -> Result<Json<Holding>, ApiError> {
    let owner_id = user.owner_id();

    let symbol = normalise_symbol(&payload.symbol).map_err(ApiError::bad_request)?;
    let currency = normalise_currency(&payload.currency)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let price_minor = parse_amount(&payload.price, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let quantity_minor = parse_amount(&payload.quantity, QUANTITY_EXPONENT)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let note = note_or_reject(&payload.note)?;

    let outcome = PortfolioRepository::buy(
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
    .map_err(|e| ApiError::internal(e.to_string()))?;

    match outcome {
        BuyOutcome::Ok { holding, cash } => {
            state.broadcaster.send(SystemEvent::PortfolioHoldingCreated(holding.clone()));
            state.broadcaster.send(cash_event(&owner_id, &cash));
            Ok(Json(holding))
        }
        BuyOutcome::CurrencyMismatch { existing_currency } => Err(ApiError::bad_request(format!(
            "{} is already held in {}",
            symbol, existing_currency
        ))),
        BuyOutcome::InsufficientCash { available_minor, needed_minor } => {
            Err(ApiError::bad_request(format!(
                "Need {} {} but only {} {} in cash",
                format_for_error(needed_minor, exponent),
                currency,
                format_for_error(available_minor, exponent),
                currency
            )))
        }
    }
}

/// Renders minor units for an error message. Reuses the same decimal shape the
/// rest of the API uses, without pulling `format_amount` into a public error
/// path's happy-path imports.
fn format_for_error(minor: i64, exponent: u32) -> String {
    crate::modules::finance::money::format_amount(minor, exponent)
}

fn cash_event(owner_id: &str, cash: &CashBalance) -> SystemEvent {
    SystemEvent::PortfolioCashUpdated {
        owner_id: owner_id.to_string(),
        currency: cash.currency.clone(),
        balance_minor: cash.balance_minor,
    }
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
    security(("bearer_auth" = []))
)]
pub async fn sell_holding(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<SellHoldingRequest>,
) -> Result<Json<SellResponse>, ApiError> {
    let owner_id = user.owner_id();

    let existing = PortfolioRepository::find_owned(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    let exponent = existing.currency_exponent as u32;
    let price_minor = parse_amount(&payload.price, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let quantity_minor = parse_amount(&payload.quantity, QUANTITY_EXPONENT)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let outcome = PortfolioRepository::sell(
        &state.db.pool,
        &owner_id,
        &existing.symbol,
        quantity_minor,
        price_minor,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    match outcome {
        SellOutcome::NotFound => Err(ApiError::not_found("Holding not found or access denied")),
        SellOutcome::ExceedsHolding { held_minor } => Err(ApiError::bad_request(format!(
            "Only {} held (minor units)",
            held_minor
        ))),
        SellOutcome::Closed { realized_pl_minor, cash } => {
            state.broadcaster.send(SystemEvent::PortfolioHoldingDeleted { id, owner_id: owner_id.clone() });
            state.broadcaster.send(cash_event(&owner_id, &cash));
            Ok(Json(SellResponse { holding: None, realized_pl_minor }))
        }
        SellOutcome::Reduced { holding, realized_pl_minor, cash } => {
            state.broadcaster.send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
            state.broadcaster.send(cash_event(&owner_id, &cash));
            Ok(Json(SellResponse { holding: Some(holding), realized_pl_minor }))
        }
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
    security(("bearer_auth" = []))
)]
pub async fn set_price(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<SetPriceRequest>,
) -> Result<Json<Holding>, ApiError> {
    let owner_id = user.owner_id();

    let existing = PortfolioRepository::find_owned(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    let exponent = existing.currency_exponent as u32;
    let price_minor = parse_amount(&payload.price, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let holding = PortfolioRepository::set_price(&state.db.pool, &owner_id, &existing.symbol, price_minor)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    state.broadcaster.send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
    Ok(Json(holding))
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
    security(("bearer_auth" = []))
)]
pub async fn set_note(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<SetNoteRequest>,
) -> Result<Json<Holding>, ApiError> {
    let owner_id = user.owner_id();
    let note = note_or_reject(&payload.note)?;

    let existing = PortfolioRepository::find_owned(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    let holding = PortfolioRepository::set_note(&state.db.pool, &owner_id, &existing.symbol, note)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    state.broadcaster.send(SystemEvent::PortfolioHoldingUpdated(holding.clone()));
    Ok(Json(holding))
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
    security(("bearer_auth" = []))
)]
pub async fn delete_holding(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let owner_id = user.owner_id();

    let existing = PortfolioRepository::find_owned(&state.db.pool, id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
        .ok_or_else(|| ApiError::not_found("Holding not found or access denied"))?;

    PortfolioRepository::delete(&state.db.pool, &owner_id, &existing.symbol)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(SystemEvent::PortfolioHoldingDeleted { id, owner_id });
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/summary",
    // `utoipa::path` defaults the operation id to the function name, and
    // `finance::summary` already claims plain "summary" — an explicit id keeps
    // the generated OpenAPI operations (and so the TS client) from colliding.
    operation_id = "portfolio_summary",
    responses(
        (status = 200, description = "Per-currency totals", body = Vec<PortfolioCurrencySummary>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn summary(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<PortfolioCurrencySummary>>, ApiError> {
    let summaries = PortfolioRepository::summary(&state.db.pool, &user.owner_id())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(summaries))
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/cash",
    responses(
        (status = 200, description = "Your cash balances", body = Vec<CashBalance>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_cash(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<CashBalance>>, ApiError> {
    let balances = PortfolioRepository::list_cash(&state.db.pool, &user.owner_id())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(balances))
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/cash/deposit",
    request_body = CashAmountRequest,
    responses(
        (status = 200, description = "Cash deposited", body = CashBalance),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn deposit_cash(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CashAmountRequest>,
) -> Result<Json<CashBalance>, ApiError> {
    let owner_id = user.owner_id();
    let (amount_minor, currency, exponent) = prepare_cash_amount(&payload)?;

    let cash = PortfolioRepository::deposit_cash(&state.db.pool, &owner_id, &currency, exponent, amount_minor)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(cash_event(&owner_id, &cash));
    Ok(Json(cash))
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/cash/withdraw",
    request_body = CashAmountRequest,
    responses(
        (status = 200, description = "Cash withdrawn", body = CashBalance),
        (status = 400, description = "Bad request, or insufficient funds", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn withdraw_cash(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CashAmountRequest>,
) -> Result<Json<CashBalance>, ApiError> {
    let owner_id = user.owner_id();
    let (amount_minor, currency, exponent) = prepare_cash_amount(&payload)?;

    match PortfolioRepository::withdraw_cash(&state.db.pool, &owner_id, &currency, amount_minor)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?
    {
        WithdrawOutcome::Ok(cash) => {
            state.broadcaster.send(cash_event(&owner_id, &cash));
            Ok(Json(cash))
        }
        WithdrawOutcome::InsufficientFunds { available_minor } => Err(ApiError::bad_request(format!(
            "Only {} {} available",
            format_for_error(available_minor, exponent as u32),
            currency
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
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    /// Buying now requires cash on hand; this deposits comfortably more than
    /// any single test spends.
    async fn fund(state: &ApiState, user: &AuthUser, currency: &str, exponent: i64) {
        PortfolioRepository::deposit_cash(&state.db.pool, &user.owner_id(), currency, exponent, 100_000_000)
            .await
            .unwrap();
    }

    fn buy(symbol: &str, qty: &str, price: &str, currency: &str) -> BuyHoldingRequest {
        BuyHoldingRequest {
            symbol: symbol.to_string(),
            quantity: qty.to_string(),
            price: price.to_string(),
            currency: currency.to_string(),
            note: None,
        }
    }

    /// Quantities and prices cross the wire as strings and are stored as minor
    /// units, so a value like 10.5 shares cannot be rounded on the way in.
    #[tokio::test]
    async fn test_buy_stores_exact_minor_units() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let holding =
            buy_holding(State(state.clone()), alice, Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();

        assert_eq!(holding.quantity_minor, 10 * 10i64.pow(QUANTITY_EXPONENT));
        assert_eq!(holding.avg_cost_minor, 15000);
        assert_eq!(holding.currency, "USD");
    }

    #[tokio::test]
    async fn test_buying_more_blends_the_average_cost() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
            .await
            .unwrap();
        let holding =
            buy_holding(State(state.clone()), alice, Json(buy("AAPL", "5", "180.00", "USD")))
                .await
                .unwrap();

        assert_eq!(holding.avg_cost_minor, 16000);
    }

    #[tokio::test]
    async fn test_buying_in_a_different_currency_is_rejected() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;
        fund(&state, &alice, "EUR", 2).await;

        buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
            .await
            .unwrap();

        let err = buy_holding(State(state), alice, Json(buy("AAPL", "5", "180.00", "EUR")))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    /// The whole point of tracking cash: a buy with nothing deposited must be
    /// rejected, not silently succeed.
    #[tokio::test]
    async fn test_buying_without_enough_cash_is_rejected() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");

        let err = buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("1500.00"), "should state the shortfall: {}", err.message);

        assert_eq!(list_holdings(State(state), alice).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_rejects_bad_amounts_and_currencies() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        for (qty, price, currency) in [("-5", "150.00", "USD"), ("10", "abc", "USD"), ("10", "150.00", "EURO")]
        {
            let err = buy_holding(
                State(state.clone()),
                alice.clone(),
                Json(buy("AAPL", qty, price, currency)),
            )
            .await
            .unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "{qty} {price} {currency} must be rejected");
        }
    }

    #[tokio::test]
    async fn test_holdings_are_private_to_their_owner() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        let bob = AuthUser::local("bob");
        fund(&state, &alice, "USD", 2).await;

        let created =
            buy_holding(State(state.clone()), alice, Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();

        let bobs_view = list_holdings(State(state.clone()), bob.clone()).await.unwrap();
        assert_eq!(bobs_view.len(), 0);

        let err = delete_holding(State(state.clone()), bob.clone(), Path(created.id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        let err = sell_holding(
            State(state),
            bob,
            Path(created.id),
            Json(SellHoldingRequest { quantity: "1".to_string(), price: "160.00".to_string() }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_sell_exceeding_the_holding_is_rejected() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let created =
            buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();

        let err = sell_holding(
            State(state),
            alice,
            Path(created.id),
            Json(SellHoldingRequest { quantity: "11".to_string(), price: "180.00".to_string() }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_partial_sell_returns_the_remaining_holding_and_a_realized_pl() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let created =
            buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();

        let result = sell_holding(
            State(state),
            alice,
            Path(created.id),
            Json(SellHoldingRequest { quantity: "4".to_string(), price: "180.00".to_string() }),
        )
        .await
        .unwrap();

        assert!(result.holding.is_some());
        assert_eq!(result.holding.as_ref().unwrap().quantity_minor, 6 * 10i64.pow(QUANTITY_EXPONENT));
        assert_eq!(result.realized_pl_minor, 12000);
    }

    #[tokio::test]
    async fn test_full_sell_closes_the_position() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let created =
            buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();

        let result = sell_holding(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(SellHoldingRequest { quantity: "10".to_string(), price: "180.00".to_string() }),
        )
        .await
        .unwrap();

        assert!(result.holding.is_none(), "a full sell must report the position as closed");
        assert_eq!(result.realized_pl_minor, 30000);

        let listed = list_holdings(State(state), alice).await.unwrap();
        assert_eq!(listed.len(), 0);
    }

    /// A sale's proceeds must land in cash, whether it closed the position or
    /// only reduced it.
    #[tokio::test]
    async fn test_selling_credits_cash_with_gross_proceeds() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let created =
            buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();
        // 100_000_000 - 150000 (cost) spent so far.
        sell_holding(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(SellHoldingRequest { quantity: "10".to_string(), price: "180.00".to_string() }),
        )
        .await
        .unwrap();

        let cash = list_cash(State(state), alice).await.unwrap();
        let usd = cash.iter().find(|c| c.currency == "USD").unwrap();
        assert_eq!(usd.balance_minor, 100_000_000 - 150000 + 180000);
    }

    #[tokio::test]
    async fn test_set_price_and_note_round_trip() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let created =
            buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();

        let priced = set_price(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(SetPriceRequest { price: "180.00".to_string() }),
        )
        .await
        .unwrap();
        assert_eq!(priced.current_price_minor, Some(18000));

        let noted = set_note(
            State(state.clone()),
            alice.clone(),
            Path(created.id),
            Json(SetNoteRequest { note: Some("core position".to_string()) }),
        )
        .await
        .unwrap();
        assert_eq!(noted.note.as_deref(), Some("core position"));

        let cleared = set_note(
            State(state),
            alice,
            Path(created.id),
            Json(SetNoteRequest { note: None }),
        )
        .await
        .unwrap();
        assert!(cleared.note.is_none());
    }

    #[tokio::test]
    async fn test_summary_separates_currencies() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;
        fund(&state, &alice, "JPY", 0).await;

        let aapl =
            buy_holding(State(state.clone()), alice.clone(), Json(buy("AAPL", "10", "150.00", "USD")))
                .await
                .unwrap();
        buy_holding(State(state.clone()), alice.clone(), Json(buy("SONY", "100", "2000", "JPY")))
            .await
            .unwrap();
        set_price(
            State(state.clone()),
            alice.clone(),
            Path(aapl.id),
            Json(SetPriceRequest { price: "180.00".to_string() }),
        )
        .await
        .unwrap();

        let summaries = summary(State(state), alice).await.unwrap();
        assert_eq!(summaries.len(), 2);

        let usd = summaries.iter().find(|s| s.currency == "USD").unwrap();
        assert_eq!(usd.market_value_minor, 180000);
        assert_eq!(usd.priced_holdings, 1);
        assert_eq!(usd.cash_minor, 100_000_000 - 150000);

        let jpy = summaries.iter().find(|s| s.currency == "JPY").unwrap();
        assert_eq!(jpy.unpriced_holdings, 1);
    }

    /// A deposit with nothing bought yet must still show up in the summary.
    #[tokio::test]
    async fn test_summary_includes_a_cash_only_currency() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");

        deposit_cash(
            State(state.clone()),
            alice.clone(),
            Json(CashAmountRequest { amount: "500.00".to_string(), currency: "GBP".to_string() }),
        )
        .await
        .unwrap();

        let summaries = summary(State(state), alice).await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].currency, "GBP");
        assert_eq!(summaries[0].cash_minor, 50000);
    }

    #[tokio::test]
    async fn test_note_exceeding_the_length_limit_is_rejected() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        fund(&state, &alice, "USD", 2).await;

        let mut request = buy("AAPL", "10", "150.00", "USD");
        request.note = Some("x".repeat(MAX_NOTE_LEN + 1));

        let err = buy_holding(State(state), alice, Json(request)).await.unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    // ---- cash ------------------------------------------------------------

    #[tokio::test]
    async fn test_deposit_then_list() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");

        let cash = deposit_cash(
            State(state.clone()),
            alice.clone(),
            Json(CashAmountRequest { amount: "1000.00".to_string(), currency: "usd".to_string() }),
        )
        .await
        .unwrap();
        assert_eq!(cash.balance_minor, 100000);
        assert_eq!(cash.currency, "USD", "should normalise like every other currency field");

        let listed = list_cash(State(state), alice).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].balance_minor, 100000);
    }

    #[tokio::test]
    async fn test_withdraw_reduces_the_balance() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        deposit_cash(
            State(state.clone()),
            alice.clone(),
            Json(CashAmountRequest { amount: "1000.00".to_string(), currency: "USD".to_string() }),
        )
        .await
        .unwrap();

        let cash = withdraw_cash(
            State(state),
            alice,
            Json(CashAmountRequest { amount: "400.00".to_string(), currency: "USD".to_string() }),
        )
        .await
        .unwrap();
        assert_eq!(cash.balance_minor, 60000);
    }

    #[tokio::test]
    async fn test_withdraw_exceeding_the_balance_is_rejected() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        deposit_cash(
            State(state.clone()),
            alice.clone(),
            Json(CashAmountRequest { amount: "100.00".to_string(), currency: "USD".to_string() }),
        )
        .await
        .unwrap();

        let err = withdraw_cash(
            State(state),
            alice,
            Json(CashAmountRequest { amount: "100.01".to_string(), currency: "USD".to_string() }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
        assert!(err.message.contains("100.00"));
    }

    #[tokio::test]
    async fn test_cash_is_private_to_its_owner() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        let bob = AuthUser::local("bob");
        deposit_cash(
            State(state.clone()),
            alice,
            Json(CashAmountRequest { amount: "1000.00".to_string(), currency: "USD".to_string() }),
        )
        .await
        .unwrap();

        assert_eq!(list_cash(State(state), bob).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn test_deposit_broadcasts_to_the_owner() {
        let state = test_state().await;
        let alice = AuthUser::local("alice");
        let mut rx = state.broadcaster.subscribe();

        deposit_cash(
            State(state),
            alice,
            Json(CashAmountRequest { amount: "1000.00".to_string(), currency: "USD".to_string() }),
        )
        .await
        .unwrap();

        let event = rx.try_recv().expect("a deposit must be broadcast");
        assert_eq!(event.owner_id(), "@web-alice:api");
        assert!(matches!(event, SystemEvent::PortfolioCashUpdated { .. }));
    }
}
