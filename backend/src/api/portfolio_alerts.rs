use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::SystemEvent;
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::portfolio::alerts::{AlertCondition, AlertsRepository, PriceAlert};
use crate::modules::portfolio::models::normalise_symbol;

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
    let alerts = AlertsRepository::list_for_owner(state.db.sqlite_pool(), &user.owner_id())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(alerts))
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
    let exponent = exponent_for(&currency);

    let target_price_minor = parse_amount(&payload.target_price, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    if target_price_minor <= 0 {
        return Err(ApiError::bad_request("Target price must be positive"));
    }

    let owner_id = user.owner_id();
    let alert = AlertsRepository::create(
        state.db.sqlite_pool(),
        &owner_id,
        &symbol,
        target_price_minor,
        &currency,
        exponent as i32,
        payload.condition,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    state
        .broadcaster
        .send(SystemEvent::PortfolioAlertCreated(alert.clone()));

    Ok((StatusCode::CREATED, Json(alert)))
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
    let owner_id = user.owner_id();

    let deleted = AlertsRepository::delete(state.db.sqlite_pool(), id, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    if !deleted {
        return Err(ApiError::not_found("Alert not found or access denied"));
    }

    state
        .broadcaster
        .send(SystemEvent::PortfolioAlertDeleted { id, owner_id });

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;

    async fn setup_state() -> (ApiState, AuthUser) {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&crate::db::DbPool::Sqlite(pool.clone()))
            .await
            .unwrap();
        let broadcaster = EventBroadcaster::default();
        let state = ApiState::new(
            crate::db::Database {
                pool: crate::db::DbPool::Sqlite(pool),
            },
            broadcaster,
            None,
            "http://localhost:8008".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser {
            account_id: "user_test".to_string(),
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
