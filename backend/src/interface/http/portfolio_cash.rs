//! REST API endpoints for portfolio cash operations (list, deposit, withdraw).

use axum::{extract::State, Json};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::domain::portfolio::WithdrawOutcome;
use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::portfolio::{format_for_error, CashBalance};

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
    let balances = state.services.portfolio.list_cash(&user.account_id).await?;

    Ok(Json(balances.into_iter().map(CashBalance::from).collect()))
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
    let (amount_minor, currency, exponent) = prepare_cash_amount(&payload)?;

    let cash = state
        .services
        .portfolio
        .deposit_cash(
            &user.account_id,
            &currency,
            exponent,
            amount_minor,
            payload.executed_at,
        )
        .await?;

    Ok(Json(cash.into()))
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
    let (amount_minor, currency, exponent) = prepare_cash_amount(&payload)?;

    match state
        .services
        .portfolio
        .withdraw_cash(
            &user.account_id,
            &currency,
            exponent,
            amount_minor,
            payload.executed_at,
        )
        .await?
    {
        WithdrawOutcome::Ok(cash) => Ok(Json(cash.into())),
        WithdrawOutcome::InsufficientFunds { available_minor } => {
            Err(ApiError::bad_request(format!(
                "Only {} {} available",
                format_for_error(available_minor, exponent as u32),
                currency
            )))
        }
    }
}
