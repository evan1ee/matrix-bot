//! API handler for unified cross-currency net worth calculation.

use axum::{extract::State, Json};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::finance::net_worth::{NetWorthResponse, NetWorthService};
use crate::modules::settings::repository::SettingsRepository;

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
    let owner_id = user.owner_id();
    let settings = SettingsRepository::get_or_default(state.db.sqlite_pool(), &owner_id).await;

    let net_worth = NetWorthService::compute_net_worth(
        state.db.sqlite_pool(),
        &state.http_client,
        &owner_id,
        &settings.currency,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to calculate net worth: {e}")))?;

    Ok(Json(net_worth))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_get_net_worth_api_handler() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::schema::run_migrations(&pool).await.unwrap();

        let state = ApiState::new(
            crate::db::Database {
                pool: crate::db::DbPool::Sqlite(pool),
            },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let user = AuthUser::for_account("networth_user");

        let res = get_net_worth(State(state), user).await.unwrap();
        assert_eq!(res.primary_currency, "USD");
        assert_eq!(res.total_net_worth_minor, 0);
    }
}
