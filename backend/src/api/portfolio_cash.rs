//! REST API endpoints for portfolio cash operations (list, deposit, withdraw).

use axum::{extract::State, Json};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::api::portfolio::{cash_event, format_for_error};
use crate::error::ApiError;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::portfolio::cash_repo;
use crate::modules::portfolio::models::{CashBalance, WithdrawOutcome};

#[derive(Deserialize, ToSchema)]
pub struct CashAmountRequest {
    /// Decimal string, e.g. `"1000.00"`.
    pub amount: String,
    /// ISO 4217 code, e.g. `"USD"`.
    pub currency: String,
    pub executed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Validates a `{amount, currency}` cash request, returning minor units.
pub fn prepare_cash_amount(payload: &CashAmountRequest) -> Result<(i64, String, i64), ApiError> {
    let currency =
        normalise_currency(&payload.currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(&payload.amount, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok((amount_minor, currency, exponent as i64))
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/cash",
    responses(
        (status = 200, description = "Your cash balances", body = Vec<CashBalance>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_cash(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<CashBalance>>, ApiError> {
    let balances = cash_repo::list_cash(&state.db.pool, &user.owner_id())
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
    security(("session_cookie" = []))
)]
pub async fn deposit_cash(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CashAmountRequest>,
) -> Result<Json<CashBalance>, ApiError> {
    let owner_id = user.owner_id();
    let (amount_minor, currency, exponent) = prepare_cash_amount(&payload)?;

    let cash = cash_repo::deposit_cash_at(
        &state.db.pool,
        &owner_id,
        &currency,
        exponent,
        amount_minor,
        payload.executed_at,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state.broadcaster.send(cash_event(&owner_id, &cash));
    // The service broadcasts `TransactionCreated` itself; this handler only
    // needs the `PortfolioCashUpdated` event above.
    crate::modules::portfolio::cash::record_portfolio_cash_transaction(
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
        "deposit",
        amount_minor,
        &currency,
        exponent,
        cash.id,
        payload.executed_at,
    )
    .await;
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
    security(("session_cookie" = []))
)]
pub async fn withdraw_cash(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CashAmountRequest>,
) -> Result<Json<CashBalance>, ApiError> {
    let owner_id = user.owner_id();
    let (amount_minor, currency, exponent) = prepare_cash_amount(&payload)?;

    match cash_repo::withdraw_cash_at(
        &state.db.pool,
        &owner_id,
        &currency,
        amount_minor,
        payload.executed_at,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?
    {
        WithdrawOutcome::Ok(cash) => {
            state.broadcaster.send(cash_event(&owner_id, &cash));
            // The service broadcasts `TransactionCreated` itself; this
            // handler only needs the `PortfolioCashUpdated` event above.
            crate::modules::portfolio::cash::record_portfolio_cash_transaction(
                &state.db.pool,
                Some(&state.broadcaster),
                &owner_id,
                "withdraw",
                amount_minor,
                &currency,
                exponent,
                cash.id,
                payload.executed_at,
            )
            .await;
            Ok(Json(cash))
        }
        WithdrawOutcome::InsufficientFunds { available_minor } => {
            Err(ApiError::bad_request(format!(
                "Only {} {} available",
                format_for_error(available_minor, exponent as u32),
                currency
            )))
        }
    }
}
