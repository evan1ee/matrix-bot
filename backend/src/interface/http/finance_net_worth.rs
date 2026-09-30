//! API handler for unified cross-currency net worth calculation.

use axum::{extract::State, Json};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CurrencyNetWorthBreakdown {
    pub currency: String,
    pub currency_exponent: i64,
    pub ledger_cash_minor: i64,
    pub portfolio_market_value_minor: i64,
    pub cash_deposits_minor: i64,
    pub total_minor: i64,
    pub fx_rate_to_primary: f64,
    pub converted_minor: i64,
}

impl From<crate::domain::finance::net_worth::CurrencyNetWorthBreakdown>
    for CurrencyNetWorthBreakdown
{
    fn from(c: crate::domain::finance::net_worth::CurrencyNetWorthBreakdown) -> Self {
        Self {
            currency: c.currency,
            currency_exponent: c.currency_exponent,
            ledger_cash_minor: c.ledger_cash_minor,
            portfolio_market_value_minor: c.portfolio_market_value_minor,
            cash_deposits_minor: c.cash_deposits_minor,
            total_minor: c.total_minor,
            fx_rate_to_primary: c.fx_rate_to_primary,
            converted_minor: c.converted_minor,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct NetWorthResponse {
    pub primary_currency: String,
    pub primary_currency_exponent: i64,
    pub total_net_worth_minor: i64,
    pub ledger_cash_minor: i64,
    pub portfolio_market_value_minor: i64,
    pub cash_deposits_minor: i64,
    pub currencies: Vec<CurrencyNetWorthBreakdown>,
    pub fx_rates: HashMap<String, f64>,
    pub updated_at: DateTime<Utc>,
}

impl From<crate::domain::finance::net_worth::NetWorthResponse> for NetWorthResponse {
    fn from(n: crate::domain::finance::net_worth::NetWorthResponse) -> Self {
        Self {
            primary_currency: n.primary_currency,
            primary_currency_exponent: n.primary_currency_exponent,
            total_net_worth_minor: n.total_net_worth_minor,
            ledger_cash_minor: n.ledger_cash_minor,
            portfolio_market_value_minor: n.portfolio_market_value_minor,
            cash_deposits_minor: n.cash_deposits_minor,
            currencies: n
                .currencies
                .into_iter()
                .map(CurrencyNetWorthBreakdown::from)
                .collect(),
            fx_rates: n.fx_rates,
            updated_at: n.updated_at,
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/net-worth",
    responses(
        (status = 200, description = "Unified estimated net worth across ledger, portfolio, and cash deposits", body = NetWorthResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn get_net_worth(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<NetWorthResponse>, ApiError> {
    let settings = state
        .services
        .settings
        .get_or_default(&user.account_id)
        .await;

    let net_worth = crate::infrastructure::postgres::read_models::net_worth::compute_net_worth(
        &state.db.pool,
        &state.http_client,
        &user.account_id,
        &settings.currency,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to calculate net worth: {e}")))?;

    Ok(Json(net_worth.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_net_worth_api_handler() {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let user = AuthUser::for_seed("networth_user");

        let res = get_net_worth(State(state), user).await.unwrap();
        assert_eq!(res.primary_currency, "USD");
        assert_eq!(res.total_net_worth_minor, 0);
    }
}
