use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct MonthlyCashflowBucket {
    pub month: String,
    pub income_minor: i64,
    pub expense_minor: i64,
    pub net_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
}

impl From<crate::domain::finance::analytics::MonthlyCashflowBucket> for MonthlyCashflowBucket {
    fn from(b: crate::domain::finance::analytics::MonthlyCashflowBucket) -> Self {
        Self {
            month: b.month,
            income_minor: b.income_minor,
            expense_minor: b.expense_minor,
            net_minor: b.net_minor,
            currency: b.currency,
            currency_exponent: b.currency_exponent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct ExpenseCategorySlice {
    pub category_name: String,
    pub amount_minor: i64,
    pub percentage: f64,
    pub tx_count: i64,
    pub currency: String,
    pub currency_exponent: i64,
}

impl From<crate::domain::finance::analytics::ExpenseCategorySlice> for ExpenseCategorySlice {
    fn from(s: crate::domain::finance::analytics::ExpenseCategorySlice) -> Self {
        Self {
            category_name: s.category_name,
            amount_minor: s.amount_minor,
            percentage: s.percentage,
            tx_count: s.tx_count,
            currency: s.currency,
            currency_exponent: s.currency_exponent,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct FinanceAnalyticsReport {
    pub currency: String,
    pub currency_exponent: i64,
    pub months_requested: i64,
    pub total_income_minor: i64,
    pub total_expense_minor: i64,
    pub net_savings_minor: i64,
    pub monthly_cashflow: Vec<MonthlyCashflowBucket>,
    pub expense_categories: Vec<ExpenseCategorySlice>,
}

impl From<crate::domain::finance::analytics::FinanceAnalyticsReport> for FinanceAnalyticsReport {
    fn from(r: crate::domain::finance::analytics::FinanceAnalyticsReport) -> Self {
        Self {
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            months_requested: r.months_requested,
            total_income_minor: r.total_income_minor,
            total_expense_minor: r.total_expense_minor,
            net_savings_minor: r.net_savings_minor,
            monthly_cashflow: r
                .monthly_cashflow
                .into_iter()
                .map(MonthlyCashflowBucket::from)
                .collect(),
            expense_categories: r
                .expense_categories
                .into_iter()
                .map(ExpenseCategorySlice::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PublicTransaction {
    pub id: i64,
    pub direction: String,
    pub amount_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
    pub category_name: Option<String>,
    pub note: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

impl From<crate::domain::finance::analytics::PublicTransaction> for PublicTransaction {
    fn from(t: crate::domain::finance::analytics::PublicTransaction) -> Self {
        Self {
            id: t.id,
            direction: t.direction,
            amount_minor: t.amount_minor,
            currency: t.currency,
            currency_exponent: t.currency_exponent,
            category_name: t.category_name,
            note: t.note,
            occurred_at: t.occurred_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema)]
pub struct PublicFinanceReport {
    pub title: String,
    pub currency: String,
    pub currency_exponent: i64,
    pub total_income_minor: i64,
    pub total_expense_minor: i64,
    pub net_savings_minor: i64,
    pub monthly_cashflow: Vec<MonthlyCashflowBucket>,
    pub expense_categories: Vec<ExpenseCategorySlice>,
    pub recent_transactions: Vec<PublicTransaction>,
}

impl From<crate::domain::finance::analytics::PublicFinanceReport> for PublicFinanceReport {
    fn from(r: crate::domain::finance::analytics::PublicFinanceReport) -> Self {
        Self {
            title: r.title,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            total_income_minor: r.total_income_minor,
            total_expense_minor: r.total_expense_minor,
            net_savings_minor: r.net_savings_minor,
            monthly_cashflow: r
                .monthly_cashflow
                .into_iter()
                .map(MonthlyCashflowBucket::from)
                .collect(),
            expense_categories: r
                .expense_categories
                .into_iter()
                .map(ExpenseCategorySlice::from)
                .collect(),
            recent_transactions: r
                .recent_transactions
                .into_iter()
                .map(PublicTransaction::from)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FinanceShare {
    pub owner_id: String,
    pub token: String,
    pub is_enabled: bool,
    pub title: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::finance::FinanceShare> for FinanceShare {
    fn from(s: crate::domain::finance::FinanceShare) -> Self {
        Self {
            owner_id: s.owner_id.into_inner(),
            token: s.token,
            is_enabled: s.is_enabled,
            title: s.title,
            created_at: s.created_at,
            updated_at: s.updated_at,
        }
    }
}

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
    let report = state
        .services
        .finance
        .get_analytics_report(&auth.account_id, query.currency.as_deref(), months)
        .await?;

    Ok(Json(report.into()))
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
    let share = state.services.finance.get_share(&auth.account_id).await?;

    Ok(Json(share.map(Into::into)))
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
    let share = state
        .services
        .finance
        .upsert_share(
            &auth.account_id,
            payload.is_enabled,
            payload.title.as_deref(),
        )
        .await?;

    Ok(Json(share.into()))
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
    state
        .services
        .finance
        .disable_share(&auth.account_id)
        .await?;

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
    let report = state
        .services
        .finance
        .get_public_report(&token, months)
        .await?;

    match report {
        Some(r) => Ok(Json(r.into())),
        None => Err(ApiError::not_found(
            "Public fiscal transparency report not found or link has expired",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::finance::{Direction, NewTransaction};
    use crate::events::EventBroadcaster;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
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

        // Create sample transaction
        state
            .services
            .finance
            .create_transaction(
                &alice.account_id,
                NewTransaction {
                    direction: Direction::Income,
                    amount_minor: 100_000,
                    currency: "EUR",
                    currency_exponent: 2,
                    category: None,
                    note: Some("Stipend"),
                    room_id: "!room:1",
                    sender_name: "Alice",
                    occurred_at: crate::utils::time::now_at_storage_precision(),
                    source: None,
                    create_missing_category: false,
                },
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
