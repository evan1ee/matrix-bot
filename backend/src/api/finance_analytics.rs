use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::finance::analytics::{
    FinanceAnalyticsReport, FinanceAnalyticsRepository, FinanceShare, PublicFinanceReport,
};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct AnalyticsQuery {
    /// ISO 4217 code (e.g. "USD", "EUR"). If omitted, uses user's primary currency.
    pub currency: Option<String>,
    /// Number of trailing months to aggregate (default 6, max 36).
    pub months: Option<i64>,
}

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub struct PublicQuery {
    /// Number of trailing months to aggregate (default 6, max 36).
    pub months: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ShareRequest {
    pub is_enabled: bool,
    pub title: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatusResponse {
    pub success: bool,
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/analytics",
    tag = "finance",
    params(AnalyticsQuery),
    responses(
        (status = 200, description = "Aggregated monthly cashflow and expense breakdown", body = FinanceAnalyticsReport),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    )
)]
pub async fn get_analytics(
    State(state): State<ApiState>,
    auth: AuthUser,
    Query(query): Query<AnalyticsQuery>,
) -> Result<Json<FinanceAnalyticsReport>, ApiError> {
    let months = query.months.unwrap_or(6);
    let owner_id = auth.owner_id();
    let report = FinanceAnalyticsRepository::get_analytics_report(
        &state.db.pool,
        &owner_id,
        query.currency.as_deref(),
        months,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(report))
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/share",
    tag = "finance",
    responses(
        (status = 200, description = "Current public sharing configuration", body = Option<FinanceShare>),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn get_share(
    State(state): State<ApiState>,
    auth: AuthUser,
) -> Result<Json<Option<FinanceShare>>, ApiError> {
    let owner_id = auth.owner_id();
    let share = FinanceAnalyticsRepository::get_share(&state.db.pool, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(share))
}

#[utoipa::path(
    post,
    path = "/api/v1/finance/share",
    tag = "finance",
    request_body = ShareRequest,
    responses(
        (status = 200, description = "Configured public share", body = FinanceShare),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn upsert_share(
    State(state): State<ApiState>,
    auth: AuthUser,
    Json(payload): Json<ShareRequest>,
) -> Result<Json<FinanceShare>, ApiError> {
    let owner_id = auth.owner_id();
    let share = FinanceAnalyticsRepository::upsert_share(
        &state.db.pool,
        &owner_id,
        payload.is_enabled,
        payload.title.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(share))
}

#[utoipa::path(
    delete,
    path = "/api/v1/finance/share",
    tag = "finance",
    responses(
        (status = 200, description = "Public share disabled", body = StatusResponse),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn delete_share(
    State(state): State<ApiState>,
    auth: AuthUser,
) -> Result<Json<StatusResponse>, ApiError> {
    let owner_id = auth.owner_id();
    FinanceAnalyticsRepository::disable_share(&state.db.pool, &owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(StatusResponse { success: true }))
}

#[utoipa::path(
    get,
    path = "/api/v1/public/finance/{token}",
    tag = "finance",
    params(
        ("token" = String, Path, description = "Secure sharing token"),
        PublicQuery
    ),
    responses(
        (status = 200, description = "Public fiscal transparency report", body = PublicFinanceReport),
        (status = 404, description = "Report not found or link disabled")
    )
)]
pub async fn get_public_report(
    State(state): State<ApiState>,
    Path(token): Path<String>,
    Query(query): Query<PublicQuery>,
) -> Result<Json<PublicFinanceReport>, ApiError> {
    let months = query.months.unwrap_or(6);
    let report = FinanceAnalyticsRepository::get_public_report(&state.db.pool, &token, months)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    match report {
        Some(r) => Ok(Json(r)),
        None => Err(ApiError::not_found(
            "Public fiscal transparency report not found or link has expired",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use crate::modules::finance::repository::FinanceRepository;
    use chrono::Utc;
    use sqlx::SqlitePool;

    async fn test_state() -> ApiState {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_api_handlers_integration() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        let owner_id = alice.owner_id().to_string();

        // Create sample transaction
        FinanceRepository::create_transaction(
            &state.db.pool,
            "!room:1",
            &owner_id,
            "Alice",
            "income",
            100_000,
            "EUR",
            2,
            None,
            Some("Stipend"),
            None,
            None,
            Utc::now(),
        )
        .await
        .unwrap();

        // 1. Get analytics
        let res = get_analytics(
            State(state.clone()),
            alice.clone(),
            Query(AnalyticsQuery {
                currency: Some("EUR".to_string()),
                months: Some(3),
            }),
        )
        .await
        .unwrap();
        assert_eq!(res.currency, "EUR");
        assert_eq!(res.total_income_minor, 100_000);

        // 2. Share
        let share_res = upsert_share(
            State(state.clone()),
            alice.clone(),
            Json(ShareRequest {
                is_enabled: true,
                title: Some("Public Transparency Ledger".to_string()),
            }),
        )
        .await
        .unwrap();
        assert!(share_res.is_enabled);

        // 3. Public access
        let pub_res = get_public_report(
            State(state.clone()),
            Path(share_res.token.clone()),
            Query(PublicQuery { months: Some(3) }),
        )
        .await
        .unwrap();
        assert_eq!(pub_res.title, "Public Transparency Ledger");
        assert_eq!(pub_res.recent_transactions.len(), 1);

        // 4. Disable share
        let _ = delete_share(State(state.clone()), alice.clone())
            .await
            .unwrap();
        let disabled_res = get_public_report(
            State(state.clone()),
            Path(share_res.token.clone()),
            Query(PublicQuery { months: Some(3) }),
        )
        .await;
        assert!(disabled_res.is_err());
    }
}
