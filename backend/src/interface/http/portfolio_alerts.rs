use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::domain::portfolio::models::normalise_symbol;
use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum AlertCondition {
    Above,
    Below,
}

impl From<crate::domain::portfolio::AlertCondition> for AlertCondition {
    fn from(c: crate::domain::portfolio::AlertCondition) -> Self {
        match c {
            crate::domain::portfolio::AlertCondition::Above => AlertCondition::Above,
            crate::domain::portfolio::AlertCondition::Below => AlertCondition::Below,
        }
    }
}

impl From<AlertCondition> for crate::domain::portfolio::AlertCondition {
    fn from(c: AlertCondition) -> Self {
        match c {
            AlertCondition::Above => crate::domain::portfolio::AlertCondition::Above,
            AlertCondition::Below => crate::domain::portfolio::AlertCondition::Below,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PriceAlert {
    pub id: i64,
    pub owner_id: String,
    pub symbol: String,
    pub target_price_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub condition: String,
    pub is_triggered: bool,
    pub triggered_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl From<crate::domain::portfolio::PriceAlert> for PriceAlert {
    fn from(a: crate::domain::portfolio::PriceAlert) -> Self {
        Self {
            id: a.id,
            owner_id: a.owner_id.to_string(),
            symbol: a.symbol,
            target_price_minor: a.target_price_minor,
            currency: a.currency,
            currency_exponent: a.currency_exponent,
            condition: a.condition,
            is_triggered: a.is_triggered,
            triggered_at: a.triggered_at,
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAlertRequest {
    /// Ticker or crypto pair, e.g. "BTC-USD" or "AAPL"
    pub symbol: String,
    /// Target price as decimal string, e.g. "65000.00"
    pub target_price: String,
    /// Currency, e.g. "USD". Defaults to "USD" if omitted.
    pub currency: Option<String>,
    /// Alert condition: "above" or "below"
    pub condition: AlertCondition,
}

#[utoipa::path(
    get,
    path = "/api/v1/portfolio/alerts",
    responses(
        (status = 200, description = "List of price alerts", body = Vec<PriceAlert>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_alerts(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<PriceAlert>>, ApiError> {
    let alerts = state
        .services
        .portfolio
        .list_alerts(&user.account_id)
        .await?;

    Ok(Json(alerts.into_iter().map(PriceAlert::from).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/portfolio/alerts",
    request_body = CreateAlertRequest,
    responses(
        (status = 201, description = "Alert created", body = PriceAlert),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_alert(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateAlertRequest>,
) -> Result<(StatusCode, Json<PriceAlert>), ApiError> {
    let symbol =
        normalise_symbol(&payload.symbol).map_err(|e| ApiError::bad_request(e.to_string()))?;

    let raw_currency = payload.currency.as_deref().unwrap_or("USD");
    let currency =
        normalise_currency(raw_currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = i64::from(exponent_for(&currency));

    let target_price_minor = parse_amount(&payload.target_price, exponent as u32)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    if target_price_minor <= 0 {
        return Err(ApiError::bad_request("Target price must be positive"));
    }

    let alert = state
        .services
        .portfolio
        .create_alert(
            &user.account_id,
            &symbol,
            target_price_minor,
            &currency,
            exponent,
            payload.condition.into(),
        )
        .await?;

    Ok((StatusCode::CREATED, Json(alert.into())))
}

#[utoipa::path(
    delete,
    path = "/api/v1/portfolio/alerts/{id}",
    params(("id" = i64, Path, description = "Alert ID")),
    responses(
        (status = 204, description = "Alert deleted"),
        (status = 404, description = "Alert not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_alert(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let deleted = state
        .services
        .portfolio
        .delete_alert(&user.account_id, id)
        .await?;

    if !deleted {
        return Err(ApiError::not_found("Alert not found or access denied"));
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;

    async fn setup_state() -> (ApiState, AuthUser) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["user_test"]).await;
        let broadcaster = EventBroadcaster::default();
        let state = ApiState::new(
            crate::db::Database { pool },
            broadcaster,
            None,
            "http://localhost:8008".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser {
            account_id: crate::domain::accounts::AccountId::from_seed("user_test"),
            display_name: "Test User".to_string(),
            session_id_hash: "hash".to_string(),
            last_verified_at: chrono::Utc::now(),
            restricted_until: None,
        };
        (state, user)
    }

    #[tokio::test]
    async fn test_create_and_list_alerts_api() {
        let (state, user) = setup_state().await;

        let req = CreateAlertRequest {
            symbol: "AAPL".to_string(),
            target_price: "250.50".to_string(),
            currency: Some("USD".to_string()),
            condition: AlertCondition::Above,
        };

        let (status, Json(created)) = create_alert(State(state.clone()), user.clone(), Json(req))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created.symbol, "AAPL");
        assert_eq!(created.target_price_minor, 25050);

        let Json(list) = list_alerts(State(state.clone()), user.clone())
            .await
            .unwrap();
        assert_eq!(list.len(), 1);

        let del_status = delete_alert(State(state.clone()), user.clone(), Path(created.id))
            .await
            .unwrap();
        assert_eq!(del_status, StatusCode::NO_CONTENT);

        let Json(list_after) = list_alerts(State(state.clone()), user.clone())
            .await
            .unwrap();
        assert!(list_after.is_empty());
    }
}
