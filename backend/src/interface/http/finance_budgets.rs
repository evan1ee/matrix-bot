use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::{
    Budget, BudgetPatch, BudgetScope, BudgetState, BudgetStatus, Month, NewBudget, Pace,
};
use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

const MAX_FIELD_LEN: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScopeDto {
    Overall,
    Category,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStateDto {
    Ok,
    Warning,
    Exceeded,
}

impl From<BudgetState> for BudgetStateDto {
    fn from(s: BudgetState) -> Self {
        match s {
            BudgetState::Ok => Self::Ok,
            BudgetState::Warning => Self::Warning,
            BudgetState::Exceeded => Self::Exceeded,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BudgetDto {
    pub id: i64,
    pub scope: BudgetScopeDto,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub currency: String,
    pub currency_exponent: i64,
    pub limit_minor: i64,
    pub alerts_enabled: bool,
}

impl From<&Budget> for BudgetDto {
    fn from(b: &Budget) -> Self {
        let (scope, category_id, category_name) = match &b.scope {
            BudgetScope::Overall => (BudgetScopeDto::Overall, None, None),
            BudgetScope::Category { id, name } => {
                (BudgetScopeDto::Category, Some(*id), Some(name.clone()))
            }
        };
        Self {
            id: b.id,
            scope,
            category_id,
            category_name,
            currency: b.currency.clone(),
            currency_exponent: b.currency_exponent,
            limit_minor: b.limit_minor,
            alerts_enabled: b.alerts_enabled,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PaceDto {
    pub expected_minor: i64,
    pub projected_minor: i64,
}

impl From<&Pace> for PaceDto {
    fn from(p: &Pace) -> Self {
        Self {
            expected_minor: p.expected_minor,
            projected_minor: p.projected_minor,
        }
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BudgetStatusDto {
    pub budget: BudgetDto,
    pub month: String,
    pub spent_minor: i64,
    pub remaining_minor: i64,
    pub percent_used: i64,
    pub state: BudgetStateDto,
    pub pace: Option<PaceDto>,
}

impl From<&BudgetStatus> for BudgetStatusDto {
    fn from(s: &BudgetStatus) -> Self {
        Self {
            budget: (&s.budget).into(),
            month: s.month.to_string(),
            spent_minor: s.spent_minor,
            remaining_minor: s.remaining_minor,
            percent_used: s.percent_used,
            state: s.state.into(),
            pace: s.pace.as_ref().map(PaceDto::from),
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct CreateBudgetRequest {
    pub category_id: Option<i64>,
    pub currency: String,
    pub limit: String,
    pub alerts_enabled: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateBudgetRequest {
    pub limit: Option<String>,
    pub alerts_enabled: Option<bool>,
}

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct BudgetQuery {
    pub month: Option<String>,
}

fn check_len(field: &str, value: &str) -> Result<(), ApiError> {
    if value.chars().count() > MAX_FIELD_LEN {
        return Err(ApiError::bad_request(format!(
            "{field} must be at most {MAX_FIELD_LEN} characters"
        )));
    }
    Ok(())
}

/// The caller's category name for `category_id`; another owner's id is a 404.
async fn owned_category_name(
    state: &ApiState,
    owner: &AccountId,
    category_id: Option<i64>,
) -> Result<Option<String>, ApiError> {
    let Some(id) = category_id else {
        return Ok(None);
    };
    let categories = state.services.finance.list_categories(owner).await?;
    categories
        .into_iter()
        .find(|c| c.id == id)
        .map(|c| Some(c.name))
        .ok_or_else(|| ApiError::not_found("Category not found"))
}

/// One budget measured against the owner's current month.
async fn current_status(
    state: &ApiState,
    owner: &AccountId,
    id: i64,
) -> Result<BudgetStatusDto, ApiError> {
    let statuses = state.services.finance.budget_status(owner, None).await?;
    statuses
        .iter()
        .find(|s| s.budget.id == id)
        .map(BudgetStatusDto::from)
        .ok_or_else(|| ApiError::not_found("Budget not found"))
}

fn parse_limit(limit: &str, exponent: u32) -> Result<i64, ApiError> {
    check_len("limit", limit)?;
    parse_amount(limit, exponent).map_err(|e| ApiError::bad_request(e.to_string()))
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/budgets",
    tag = "finance",
    params(BudgetQuery),
    responses(
        (status = 200, description = "Budgets measured against the month", body = Vec<BudgetStatusDto>),
        (status = 400, description = "Invalid month", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_budgets(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<BudgetQuery>,
) -> Result<Json<Vec<BudgetStatusDto>>, ApiError> {
    let month = match query.month.as_deref() {
        Some(m) => {
            check_len("month", m)?;
            Some(Month::parse(m).map_err(|e| ApiError::bad_request(e.to_string()))?)
        }
        None => None,
    };
    let statuses = state
        .services
        .finance
        .budget_status(&user.account_id, month)
        .await?;
    Ok(Json(statuses.iter().map(BudgetStatusDto::from).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/finance/budgets",
    tag = "finance",
    request_body = CreateBudgetRequest,
    responses(
        (status = 201, description = "Budget created", body = BudgetStatusDto),
        (status = 400, description = "Invalid input", body = ApiErrorResponse),
        (status = 404, description = "Unknown category", body = ApiErrorResponse),
        (status = 409, description = "A budget for that scope and currency exists", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_budget(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CreateBudgetRequest>,
) -> Result<(StatusCode, Json<BudgetStatusDto>), ApiError> {
    check_len("currency", &payload.currency)?;
    let currency =
        normalise_currency(&payload.currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let limit_minor = parse_limit(&payload.limit, exponent_for(&currency))?;
    let category = owned_category_name(&state, &user.account_id, payload.category_id).await?;
    let new = NewBudget::new(
        category.as_deref(),
        &currency,
        limit_minor,
        payload.alerts_enabled,
    )
    .map_err(|e| ApiError::bad_request(e.to_string()))?;

    let created = state
        .services
        .finance
        .create_budget(&user.account_id, new)
        .await?;
    let dto = current_status(&state, &user.account_id, created.id).await?;
    Ok((StatusCode::CREATED, Json(dto)))
}

#[utoipa::path(
    put,
    path = "/api/v1/finance/budgets/{id}",
    tag = "finance",
    params(("id" = i64, Path, description = "Budget ID")),
    request_body = UpdateBudgetRequest,
    responses(
        (status = 200, description = "Budget updated", body = BudgetStatusDto),
        (status = 400, description = "Invalid input", body = ApiErrorResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_budget(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<UpdateBudgetRequest>,
) -> Result<Json<BudgetStatusDto>, ApiError> {
    // The budget's own currency decides how many decimals the new limit may carry.
    let existing = state
        .services
        .finance
        .list_budgets(&user.account_id)
        .await?
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| ApiError::not_found("Budget not found"))?;
    let limit_minor = payload
        .limit
        .as_deref()
        .map(|l| parse_limit(l, existing.currency_exponent as u32))
        .transpose()?;

    state
        .services
        .finance
        .update_budget(
            &user.account_id,
            id,
            BudgetPatch {
                limit_minor,
                alerts_enabled: payload.alerts_enabled,
            },
        )
        .await?;
    Ok(Json(current_status(&state, &user.account_id, id).await?))
}

#[utoipa::path(
    delete,
    path = "/api/v1/finance/budgets/{id}",
    tag = "finance",
    params(("id" = i64, Path, description = "Budget ID")),
    responses(
        (status = 204, description = "Budget deleted"),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_budget(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    state
        .services
        .finance
        .delete_budget(&user.account_id, id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use axum::http::StatusCode;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice", "acct_bob"]).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    fn create_req(category_id: Option<i64>, limit: &str) -> CreateBudgetRequest {
        CreateBudgetRequest {
            category_id,
            currency: "USD".to_string(),
            limit: limit.to_string(),
            alerts_enabled: None,
        }
    }

    fn no_month() -> Query<BudgetQuery> {
        Query(BudgetQuery { month: None })
    }

    #[tokio::test]
    async fn create_returns_201_and_list_shows_it() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");

        let (status, Json(created)) = create_budget(
            State(state.clone()),
            alice.clone(),
            Json(create_req(None, "250.50")),
        )
        .await
        .unwrap();
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(created.budget.limit_minor, 25050);
        assert_eq!(created.budget.scope, BudgetScopeDto::Overall);
        assert!(created.budget.alerts_enabled);

        let Json(listed) = list_budgets(State(state), alice, no_month()).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].budget.id, created.budget.id);
        assert_eq!(listed[0].state, BudgetStateDto::Ok);
    }

    #[tokio::test]
    async fn duplicate_scope_is_a_conflict() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let _ = create_budget(
            State(state.clone()),
            alice.clone(),
            Json(create_req(None, "10")),
        )
        .await
        .unwrap();
        let err = create_budget(State(state), alice, Json(create_req(None, "20")))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn another_owners_budget_is_404_on_put_and_delete() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let bob = AuthUser::named_seed("bob", "acct_bob");
        let (_, Json(created)) =
            create_budget(State(state.clone()), alice, Json(create_req(None, "10")))
                .await
                .unwrap();
        let id = created.budget.id;

        let err = update_budget(
            State(state.clone()),
            bob.clone(),
            Path(id),
            Json(UpdateBudgetRequest {
                limit: Some("99".to_string()),
                alerts_enabled: None,
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        let err = delete_budget(State(state), bob, Path(id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_changes_limit_and_alerts_and_delete_removes() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let (_, Json(created)) = create_budget(
            State(state.clone()),
            alice.clone(),
            Json(create_req(None, "10")),
        )
        .await
        .unwrap();
        let id = created.budget.id;

        let Json(updated) = update_budget(
            State(state.clone()),
            alice.clone(),
            Path(id),
            Json(UpdateBudgetRequest {
                limit: Some("12.34".to_string()),
                alerts_enabled: Some(false),
            }),
        )
        .await
        .unwrap();
        assert_eq!(updated.budget.limit_minor, 1234);
        assert!(!updated.budget.alerts_enabled);

        let status = delete_budget(State(state.clone()), alice.clone(), Path(id))
            .await
            .unwrap();
        assert_eq!(status, StatusCode::NO_CONTENT);
        let Json(listed) = list_budgets(State(state), alice, no_month()).await.unwrap();
        assert!(listed.is_empty());
    }

    #[tokio::test]
    async fn invalid_limits_are_400() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        for bad in ["0", "-5", "abc", ""] {
            let err = create_budget(
                State(state.clone()),
                alice.clone(),
                Json(create_req(None, bad)),
            )
            .await
            .unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_REQUEST, "limit {bad:?}");
        }
    }

    #[tokio::test]
    async fn bad_month_is_400() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let err = list_budgets(
            State(state),
            alice,
            Query(BudgetQuery {
                month: Some("2026-13".to_string()),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn another_owners_category_is_404() {
        let state = test_state().await;
        let bob = AuthUser::named_seed("bob", "acct_bob");
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let cat = state
            .services
            .finance
            .create_category(&bob.account_id, "food")
            .await
            .unwrap();
        let err = create_budget(State(state), alice, Json(create_req(Some(cat.id), "10")))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn category_budget_carries_the_category_name() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let cat = state
            .services
            .finance
            .create_category(&alice.account_id, "food")
            .await
            .unwrap();
        let (_, Json(created)) =
            create_budget(State(state), alice, Json(create_req(Some(cat.id), "10")))
                .await
                .unwrap();
        assert_eq!(created.budget.scope, BudgetScopeDto::Category);
        assert_eq!(created.budget.category_id, Some(cat.id));
        assert_eq!(created.budget.category_name.as_deref(), Some("food"));
    }

    #[tokio::test]
    async fn oversized_strings_are_400() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let long = "9".repeat(501);
        let err = create_budget(
            State(state.clone()),
            alice.clone(),
            Json(create_req(None, &long)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let mut req = create_req(None, "10");
        req.currency = "U".repeat(501);
        let err = create_budget(State(state.clone()), alice.clone(), Json(req))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);

        let err = list_budgets(
            State(state),
            alice,
            Query(BudgetQuery {
                month: Some("2".repeat(501)),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }
}
