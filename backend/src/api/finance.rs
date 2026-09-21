use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::{IntoParams, ToSchema};

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::modules::finance::models::{Category, CurrencySummary, Direction, Transaction};
use crate::modules::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::modules::finance::repository::FinanceRepository;
use crate::modules::finance::service::{self, FinanceError, NewTransaction, TransactionPatch};

impl From<FinanceError> for ApiError {
    fn from(err: FinanceError) -> Self {
        match err {
            FinanceError::TransactionNotFound | FinanceError::CategoryNotFound => {
                ApiError::not_found(err.to_string())
            }
            FinanceError::CategoryExists => ApiError::conflict(err.to_string()),
            FinanceError::Db(inner) => ApiError::internal(inner.to_string()),
            other => ApiError::bad_request(other.to_string()),
        }
    }
}

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct LedgerQuery {
    /// `expense` or `income`. Omit for both.
    pub direction: Option<String>,
    pub page: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Deserialize, IntoParams, ToSchema)]
pub struct SummaryQuery {
    /// Start of the period, inclusive.
    pub from: DateTime<Utc>,
    /// End of the period, exclusive — so month boundaries do not double-count.
    pub to: DateTime<Utc>,
}

/// Amounts arrive as decimal strings, never as JSON numbers.
///
/// A JSON number is an IEEE-754 double by the time most clients have parsed it,
/// and `15.50` does not survive that exactly. The string is converted to minor
/// units by string manipulation.
#[derive(Deserialize, ToSchema)]
pub struct TransactionRequest {
    pub direction: Direction,
    /// Decimal string, e.g. `"15.50"`.
    pub amount: String,
    /// ISO 4217 code, e.g. `"EUR"`.
    pub currency: String,
    pub category_id: Option<i64>,
    pub note: Option<String>,
    /// When the money moved. Defaults to now.
    pub occurred_at: Option<DateTime<Utc>>,
    pub room_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CategoryRequest {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct LedgerResponse {
    pub items: Vec<Transaction>,
    pub page: i64,
    pub limit: i64,
    pub total: i64,
}

#[derive(Serialize, ToSchema)]
pub struct CategoryDeletedResponse {
    pub id: i64,
    /// Entries kept but left uncategorised. Deleting a category never deletes
    /// financial records.
    pub uncategorised: i64,
}

/// Validates the amount and currency, converting the amount into minor units.
/// Note-length validation now lives in `service` — every transport shares it.
fn prepare(payload: &TransactionRequest) -> Result<(i64, String, i64), ApiError> {
    let currency =
        normalise_currency(&payload.currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(&payload.amount, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok((amount_minor, currency, exponent as i64))
}

/// Confirms a category belongs to the caller before it is referenced, and
/// hands back its name — `service::create_transaction`/`update_transaction`
/// resolve a category by name, so this is what lets the REST transport keep
/// its own ownership check (an id from another account must be rejected here,
/// before the service ever sees it) while still funnelling the write through
/// the shared resolution path.
async fn owned_category(
    state: &ApiState,
    owner_id: &str,
    category_id: Option<i64>,
) -> Result<Option<String>, ApiError> {
    let Some(id) = category_id else {
        return Ok(None);
    };

    let categories = FinanceRepository::list_categories(&state.db.pool, owner_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    match categories.into_iter().find(|c| c.id == id) {
        Some(cat) => Ok(Some(cat.name)),
        None => Err(ApiError::bad_request("Unknown category")),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/transactions",
    params(LedgerQuery),
    responses(
        (status = 200, description = "Ledger entries", body = LedgerResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn list_transactions(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<LedgerResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let offset = (page - 1) * limit;
    let owner_id = user.owner_id();

    let direction = match query.direction.as_deref() {
        Some(raw) => Some(
            Direction::parse(raw)
                .ok_or_else(|| ApiError::bad_request("direction must be expense or income"))?
                .as_str(),
        ),
        None => None,
    };

    let items = FinanceRepository::list(&state.db.pool, &owner_id, direction, limit, offset)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let total = FinanceRepository::count(&state.db.pool, &owner_id, direction)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(LedgerResponse {
        items,
        page,
        limit,
        total,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/finance/transactions",
    request_body = TransactionRequest,
    responses(
        (status = 200, description = "Entry recorded", body = Transaction),
        (status = 400, description = "Bad request", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn create_transaction(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<TransactionRequest>,
) -> Result<Json<Transaction>, ApiError> {
    let owner_id = user.owner_id();
    let (amount_minor, currency, exponent) = prepare(&payload)?;
    let category = owned_category(&state, &owner_id, payload.category_id).await?;
    let room_id = payload.room_id.unwrap_or_else(|| "web".to_string());

    let tx = service::create_transaction(
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
        NewTransaction {
            direction: payload.direction.as_str(),
            amount_minor,
            currency: &currency,
            currency_exponent: exponent,
            category: category.as_deref(),
            note: payload.note.as_deref(),
            room_id: &room_id,
            sender_name: &user.display_name,
            occurred_at: payload.occurred_at.unwrap_or_else(Utc::now),
            source: None,
            // `owned_category` already resolved the category by id, so it is
            // known to exist — never create on this path.
            create_missing_category: false,
        },
    )
    .await?;

    Ok(Json(tx))
}

#[utoipa::path(
    put,
    path = "/api/v1/finance/transactions/{id}",
    params(("id" = i64, Path, description = "Transaction ID")),
    request_body = TransactionRequest,
    responses(
        (status = 200, description = "Entry updated", body = Transaction),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn update_transaction(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
    Json(payload): Json<TransactionRequest>,
) -> Result<Json<Transaction>, ApiError> {
    let owner_id = user.owner_id();
    let (amount_minor, currency, exponent) = prepare(&payload)?;
    // Ownership of the category, like ownership of the transaction, is
    // resolved here — the id came off the wire and is not authority on its own.
    let category = owned_category(&state, &owner_id, payload.category_id).await?;

    let tx = service::update_transaction(
        &state.db.pool,
        Some(&state.broadcaster),
        &owner_id,
        id,
        TransactionPatch {
            direction: Some(payload.direction.as_str()),
            amount_minor: Some(amount_minor),
            currency: Some(&currency),
            currency_exponent: Some(exponent),
            category: Some(category.as_deref()),
            note: Some(payload.note.as_deref()),
            occurred_at: Some(payload.occurred_at.unwrap_or_else(Utc::now)),
        },
    )
    .await?;

    Ok(Json(tx))
}

#[utoipa::path(
    delete,
    path = "/api/v1/finance/transactions/{id}",
    params(("id" = i64, Path, description = "Transaction ID")),
    responses(
        (status = 204, description = "Entry deleted"),
        (status = 404, description = "Not found", body = ApiErrorResponse),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn delete_transaction(
    State(state): State<ApiState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let owner_id = user.owner_id();
    service::delete_transaction(&state.db.pool, Some(&state.broadcaster), &owner_id, id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/summary",
    params(SummaryQuery),
    responses(
        (status = 200, description = "Per-currency totals", body = Vec<CurrencySummary>),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn summary(
    State(state): State<ApiState>,
    user: AuthUser,
    Query(query): Query<SummaryQuery>,
) -> Result<Json<Vec<CurrencySummary>>, ApiError> {
    if query.to <= query.from {
        return Err(ApiError::bad_request("`to` must be after `from`"));
    }

    let summaries =
        FinanceRepository::summary(&state.db.pool, &user.owner_id(), query.from, query.to)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(summaries))
}

#[utoipa::path(
    get,
    path = "/api/v1/finance/categories",
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
    let categories = FinanceRepository::list_categories(&state.db.pool, &user.owner_id())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(Json(categories))
}

#[utoipa::path(
    post,
    path = "/api/v1/finance/categories",
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
    let category = service::create_category(
        &state.db.pool,
        Some(&state.broadcaster),
        &user.owner_id(),
        &payload.name,
    )
    .await?;

    Ok(Json(category))
}

#[utoipa::path(
    delete,
    path = "/api/v1/finance/categories/{id}",
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
    let owner_id = user.owner_id();

    let uncategorised =
        service::delete_category(&state.db.pool, Some(&state.broadcaster), &owner_id, id).await?;

    Ok(Json(CategoryDeletedResponse {
        id,
        uncategorised: uncategorised as i64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;

    async fn test_state() -> ApiState {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    fn entry(amount: &str, currency: &str, category_id: Option<i64>) -> TransactionRequest {
        TransactionRequest {
            direction: Direction::Expense,
            amount: amount.to_string(),
            currency: currency.to_string(),
            category_id,
            note: Some("lunch".to_string()),
            occurred_at: None,
            room_id: None,
        }
    }

    /// Amounts cross the wire as strings and are stored as minor units, so a
    /// value like 15.50 cannot be rounded on the way in.
    #[tokio::test]
    async fn test_amount_string_becomes_exact_minor_units() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let created = create_transaction(
            State(state.clone()),
            alice,
            Json(entry("15.50", "EUR", None)),
        )
        .await
        .unwrap();

        assert_eq!(created.amount_minor, 1550);
        assert_eq!(created.currency, "EUR");
        assert_eq!(created.currency_exponent, 2);
    }

    #[tokio::test]
    async fn test_zero_decimal_currency_round_trip() {
        let state = test_state().await;
        let created = create_transaction(
            State(state.clone()),
            AuthUser::named("alice", "acct_alice"),
            Json(entry("500", "JPY", None)),
        )
        .await
        .unwrap();

        assert_eq!(created.amount_minor, 500);
        assert_eq!(created.currency_exponent, 0);
    }

    #[tokio::test]
    async fn test_rejects_bad_amounts_and_currencies() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        for (amount, currency) in [
            ("15.501", "EUR"),
            ("-5", "EUR"),
            ("abc", "EUR"),
            ("5", "EURO"),
        ] {
            let err = create_transaction(
                State(state.clone()),
                alice.clone(),
                Json(entry(amount, currency, None)),
            )
            .await
            .unwrap_err();
            assert_eq!(
                err.status,
                StatusCode::BAD_REQUEST,
                "{amount} {currency} must be rejected"
            );
        }
    }

    #[tokio::test]
    async fn test_ledger_is_private_to_its_owner() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");
        let bob = AuthUser::named("bob", "acct_bob");

        let created = create_transaction(
            State(state.clone()),
            alice,
            Json(entry("10.00", "EUR", None)),
        )
        .await
        .unwrap();

        let bobs_view = list_transactions(
            State(state.clone()),
            bob.clone(),
            Query(LedgerQuery {
                direction: None,
                page: None,
                limit: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(bobs_view.items.len(), 0);

        let err = delete_transaction(State(state.clone()), bob.clone(), Path(created.id))
            .await
            .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);

        let err = update_transaction(
            State(state.clone()),
            bob,
            Path(created.id),
            Json(entry("99.00", "EUR", None)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
    }

    /// A category id from another account must not be attachable.
    #[tokio::test]
    async fn test_cannot_borrow_another_users_category() {
        let state = test_state().await;
        let bobs_category = create_category(
            State(state.clone()),
            AuthUser::named("bob", "acct_bob"),
            Json(CategoryRequest {
                name: "food".to_string(),
            }),
        )
        .await
        .unwrap();

        let err = create_transaction(
            State(state.clone()),
            AuthUser::named("alice", "acct_alice"),
            Json(entry("10.00", "EUR", Some(bobs_category.id))),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_duplicate_category_is_a_conflict_not_a_second_row() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

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

    /// Deleting a category must keep the money and report what it touched.
    #[tokio::test]
    async fn test_deleting_a_category_keeps_the_entries() {
        let state = test_state().await;
        let alice = AuthUser::named("alice", "acct_alice");

        let category = create_category(
            State(state.clone()),
            alice.clone(),
            Json(CategoryRequest {
                name: "food".to_string(),
            }),
        )
        .await
        .unwrap();

        let _ = create_transaction(
            State(state.clone()),
            alice.clone(),
            Json(entry("10.00", "EUR", Some(category.id))),
        )
        .await
        .unwrap();

        let deleted = delete_category(State(state.clone()), alice.clone(), Path(category.id))
            .await
            .unwrap();
        assert_eq!(deleted.uncategorised, 1);

        let ledger = list_transactions(
            State(state),
            alice,
            Query(LedgerQuery {
                direction: None,
                page: None,
                limit: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(ledger.items.len(), 1, "the entry must survive");
        assert!(ledger.items[0].category_id.is_none());
    }

    #[tokio::test]
    async fn test_summary_rejects_an_inverted_period() {
        let state = test_state().await;
        let now = Utc::now();

        let err = summary(
            State(state),
            AuthUser::named("alice", "acct_alice"),
            Query(SummaryQuery {
                from: now,
                to: now - chrono::Duration::days(1),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }
}
