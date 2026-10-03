use axum::{
    extract::{Path, State},
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

/// A user-defined spending category.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Category {
    pub id: i64,
    /// The owning `owner_id`. Categories are per-user, never shared.
    pub owner_id: String,
    pub name: String,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::finance::Category> for Category {
    fn from(c: crate::domain::finance::Category) -> Self {
        Self {
            id: c.id,
            owner_id: c.owner_id.to_string(),
            name: c.name,
            created_at: c.created_at,
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct CategoryRequest {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct CategoryDeletedResponse {
    pub id: i64,
    /// Entries kept but left uncategorised. Deleting a category never deletes
    /// financial records.
    pub uncategorised: i64,
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/categories",
    tag = "finance",
    responses(
        (status = 200, description = "Your categories", body = Vec<Category>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_categories(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Json<Vec<Category>>, ApiError> {
    let categories = state
        .services
        .finance
        .list_categories(&user.account_id)
        .await?;

    Ok(Json(categories.into_iter().map(Category::from).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/finance/categories",
    tag = "finance",
    request_body = CategoryRequest,
    responses(
        (status = 200, description = "Category created", body = Category),
        (status = 409, description = "Already exists", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_category(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<CategoryRequest>,
) -> Result<Json<Category>, ApiError> {
    let category = state
        .services
        .finance
        .create_category(&user.account_id, &payload.name)
        .await?;

    Ok(Json(category.into()))
}

#[utoipa::path(
    delete,
    path = "/api/v1/finance/categories/{id}",
    tag = "finance",
    params(("id" = i64, Path, description = "Category ID")),
    responses(
        (status = 200, description = "Category deleted", body = CategoryDeletedResponse),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_category(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<Json<CategoryDeletedResponse>, ApiError> {
    let uncategorised = state
        .services
        .finance
        .delete_category(&user.account_id, id)
        .await?
        .ok_or_else(|| ApiError::not_found("Category not found"))?;

    Ok(Json(CategoryDeletedResponse {
        id,
        uncategorised: uncategorised as i64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    async fn test_state() -> ApiState {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_alice"]).await;
        ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    #[tokio::test]
    async fn test_duplicate_category_is_a_conflict_not_a_second_row() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");

        let _ = create_category(
            State(state.clone()),
            alice.clone(),
            Json(CategoryRequest {
                name: "food".to_string(),
            }),
        )
        .await
        .unwrap();

        let err = create_category(
            State(state.clone()),
            alice.clone(),
            Json(CategoryRequest {
                name: "food".to_string(),
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::CONFLICT);

        let listed = list_categories(State(state), alice).await.unwrap();
        assert_eq!(listed.len(), 1);
    }
}
