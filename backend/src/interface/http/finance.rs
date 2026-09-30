use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::domain::accounts::AccountId;
use crate::domain::finance::money::{exponent_for, normalise_currency, parse_amount};
use crate::domain::finance::{ListTransactionsFilter, NewTransaction, TransactionPatch};
use crate::error::ApiError;
use crate::interface::http::auth::{ApiState, AuthUser};

/// Which way the money moved.
///
/// `amount_minor` is always positive; this is what carries the sign, so a row
/// cannot contradict itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Expense,
    Income,
}

impl Direction {
    pub fn as_str(&self) -> &'static str {
        match self {
            Direction::Expense => "expense",
            Direction::Income => "income",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "expense" | "expenses" | "spend" | "out" => Some(Direction::Expense),
            "income" | "in" | "earn" => Some(Direction::Income),
            _ => None,
        }
    }
}

impl From<crate::domain::finance::Direction> for Direction {
    fn from(d: crate::domain::finance::Direction) -> Self {
        match d {
            crate::domain::finance::Direction::Expense => Direction::Expense,
            crate::domain::finance::Direction::Income => Direction::Income,
        }
    }
}

impl From<Direction> for crate::domain::finance::Direction {
    fn from(d: Direction) -> Self {
        match d {
            Direction::Expense => crate::domain::finance::Direction::Expense,
            Direction::Income => crate::domain::finance::Direction::Income,
        }
    }
}

pub use super::finance_categories::{
    create_category, delete_category, list_categories, Category, CategoryDeletedResponse,
    CategoryRequest,
};

/// A single ledger entry.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Transaction {
    pub id: i64,
    pub room_id: String,
    /// The sole ownership key, matching `AuthUser::owner_id()`.
    pub owner_id: String,
    /// Display only — user-controlled, and collides across identity spaces.
    pub sender_name: String,
    pub direction: String,
    /// Minor units (cents), always positive.
    pub amount_minor: i64,
    pub currency: String,
    /// Decimal places for this row's currency, stored so a later change to the
    /// currency table cannot rewrite what this row means.
    pub currency_exponent: i64,
    /// `None` means uncategorised, which is a valid and expected state.
    pub category_id: Option<i64>,
    /// Denormalised for reads so listing a ledger is one query, not N+1.
    pub category_name: Option<String>,
    pub note: Option<String>,
    /// Which module scheduled or generated this transaction (e.g. "recurring_bill", "portfolio_cash").
    pub source_kind: Option<String>,
    /// The ID of the originating entity in that module.
    pub source_id: Option<i64>,
    /// When the money moved — not when the row was written.
    pub occurred_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
}

impl From<crate::domain::finance::Transaction> for Transaction {
    fn from(t: crate::domain::finance::Transaction) -> Self {
        Self {
            id: t.id,
            room_id: t.room_id,
            owner_id: t.owner_id.to_string(),
            sender_name: t.sender_name,
            direction: t.direction.as_str().to_string(),
            amount_minor: t.amount_minor,
            currency: t.currency,
            currency_exponent: t.currency_exponent,
            category_id: t.category_id,
            category_name: t.category_name,
            note: t.note,
            source_kind: t.source_kind,
            source_id: t.source_id,
            occurred_at: t.occurred_at,
            created_at: t.created_at,
        }
    }
}

/// Per-category totals within one currency.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CategoryTotal {
    /// `None` for uncategorised entries.
    pub category: Option<String>,
    pub expense_minor: i64,
    pub income_minor: i64,
}

impl From<crate::domain::finance::CategoryTotal> for CategoryTotal {
    fn from(c: crate::domain::finance::CategoryTotal) -> Self {
        Self {
            category: c.category,
            expense_minor: c.expense_minor,
            income_minor: c.income_minor,
        }
    }
}

/// Totals for one currency. Currencies are never combined — without a rate
/// source, adding EUR to JPY would produce a meaningless number.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CurrencySummary {
    pub currency: String,
    pub currency_exponent: i64,
    pub expense_minor: i64,
    pub income_minor: i64,
    /// `income - expense`. May legitimately be negative.
    pub net_minor: i64,
    pub categories: Vec<CategoryTotal>,
}

impl From<crate::domain::finance::CurrencySummary> for CurrencySummary {
    fn from(s: crate::domain::finance::CurrencySummary) -> Self {
        Self {
            currency: s.currency,
            currency_exponent: s.currency_exponent,
            expense_minor: s.expense_minor,
            income_minor: s.income_minor,
            net_minor: s.net_minor,
            categories: s.categories.into_iter().map(CategoryTotal::from).collect(),
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

#[derive(Serialize, ToSchema)]
pub struct LedgerResponse {
    pub items: Vec<Transaction>,
    pub page: i64,
    pub limit: i64,
    pub total: i64,
}

fn prepare(payload: &TransactionRequest) -> Result<(i64, String, i64), ApiError> {
    let currency =
        normalise_currency(&payload.currency).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let exponent = exponent_for(&currency);
    let amount_minor = parse_amount(&payload.amount, exponent)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    Ok((amount_minor, currency, exponent as i64))
}

async fn owned_category(
    state: &ApiState,
    owner_id: &AccountId,
    category_id: Option<i64>,
) -> Result<Option<String>, ApiError> {
    let Some(id) = category_id else {
        return Ok(None);
    };

    let categories = state.services.finance.list_categories(owner_id).await?;

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

    let direction = match query.direction.as_deref() {
        Some(raw) => Some(
            crate::domain::finance::Direction::parse(raw)
                .ok_or_else(|| ApiError::bad_request("direction must be expense or income"))?,
        ),
        None => None,
    };

    let filter = ListTransactionsFilter {
        direction,
        limit: Some(limit),
        offset: Some(offset),
        ..Default::default()
    };

    let items = state
        .services
        .finance
        .list_transactions(&user.account_id, &filter)
        .await?;
    let total = state
        .services
        .finance
        .count_transactions(&user.account_id, direction)
        .await?;

    Ok(Json(LedgerResponse {
        items: items.into_iter().map(Transaction::from).collect(),
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
    let (amount_minor, currency, exponent) = prepare(&payload)?;
    let category = owned_category(&state, &user.account_id, payload.category_id).await?;
    let room_id = payload.room_id.unwrap_or_else(|| "web".to_string());

    let tx = state
        .services
        .finance
        .create_transaction(
            &user.account_id,
            NewTransaction {
                direction: payload.direction.into(),
                amount_minor,
                currency: &currency,
                currency_exponent: exponent,
                category: category.as_deref(),
                note: payload.note.as_deref(),
                room_id: &room_id,
                sender_name: &user.display_name,
                occurred_at: payload.occurred_at.unwrap_or_else(Utc::now),
                source: None,
                create_missing_category: false,
            },
        )
        .await?;

    Ok(Json(tx.into()))
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
    let (amount_minor, currency, exponent) = prepare(&payload)?;
    let category = owned_category(&state, &user.account_id, payload.category_id).await?;

    let tx = state
        .services
        .finance
        .update_transaction(
            &user.account_id,
            id,
            TransactionPatch {
                direction: Some(payload.direction.into()),
                amount_minor: Some(amount_minor),
                currency: Some(&currency),
                currency_exponent: Some(exponent),
                category: Some(category.as_deref()),
                note: Some(payload.note.as_deref()),
                occurred_at: Some(payload.occurred_at.unwrap_or_else(Utc::now)),
            },
        )
        .await?;

    Ok(Json(tx.into()))
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
    state
        .services
        .finance
        .delete_transaction(&user.account_id, id)
        .await?;

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

    let summaries = state
        .services
        .finance
        .summary(&user.account_id, query.from, query.to)
        .await?;

    Ok(Json(
        summaries.into_iter().map(CurrencySummary::from).collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[tokio::test]
    async fn test_amount_string_becomes_exact_minor_units() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");

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
            AuthUser::named_seed("alice", "acct_alice"),
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
        let alice = AuthUser::named_seed("alice", "acct_alice");

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
        let alice = AuthUser::named_seed("alice", "acct_alice");
        let bob = AuthUser::named_seed("bob", "acct_bob");

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

    #[tokio::test]
    async fn test_cannot_borrow_another_users_category() {
        let state = test_state().await;
        let bobs_category = create_category(
            State(state.clone()),
            AuthUser::named_seed("bob", "acct_bob"),
            Json(CategoryRequest {
                name: "food".to_string(),
            }),
        )
        .await
        .unwrap();

        let err = create_transaction(
            State(state.clone()),
            AuthUser::named_seed("alice", "acct_alice"),
            Json(entry("10.00", "EUR", Some(bobs_category.id))),
        )
        .await
        .unwrap_err();

        assert_eq!(err.status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_deleting_a_category_keeps_the_entries() {
        let state = test_state().await;
        let alice = AuthUser::named_seed("alice", "acct_alice");

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
            AuthUser::named_seed("alice", "acct_alice"),
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
