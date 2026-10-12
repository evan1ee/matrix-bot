use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::AccountId;
use crate::domain::finance::analytics::PublicTransaction;
use crate::domain::finance::FinanceShare;

#[derive(Debug, sqlx::FromRow)]
struct ShareRow {
    owner_id: uuid::Uuid,
    token: String,
    is_enabled: bool,
    title: Option<String>,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

impl From<ShareRow> for FinanceShare {
    fn from(r: ShareRow) -> Self {
        Self {
            owner_id: AccountId::from_uuid(r.owner_id),
            token: r.token,
            is_enabled: r.is_enabled,
            title: r.title,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

pub async fn primary_currency(pool: &PgPool, owner_id: &AccountId) -> Result<String, sqlx::Error> {
    let row = sqlx::query_scalar!(
        r#"
        SELECT currency
        FROM transactions
        WHERE owner_id = $1
        GROUP BY currency
        ORDER BY COUNT(*) DESC
        LIMIT 1
        "#,
        owner_id.as_uuid()
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.unwrap_or_else(|| "USD".to_string()))
}

pub struct CashflowRow {
    pub month_str: Option<String>,
    pub direction: String,
    pub total_minor: Option<i64>,
}

pub async fn monthly_cashflow_rows(
    pool: &PgPool,
    owner_id: &AccountId,
    currency: &str,
    cutoff_start: DateTime<Utc>,
    tz_name: &str,
) -> Result<Vec<CashflowRow>, sqlx::Error> {
    let rows = sqlx::query_as!(
        CashflowRow,
        r#"
        SELECT to_char(occurred_at AT TIME ZONE $4, 'YYYY-MM') AS month_str,
               direction,
               CAST(SUM(amount_minor) AS BIGINT) AS total_minor
        FROM transactions
        WHERE owner_id = $1 AND currency = $2 AND occurred_at >= $3
        GROUP BY month_str, direction
        ORDER BY month_str ASC
        "#,
        owner_id.as_uuid(),
        currency,
        cutoff_start,
        tz_name
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub struct CategorySliceRow {
    pub category_name: Option<String>,
    pub total_minor: Option<i64>,
    pub tx_count: Option<i64>,
}

pub async fn category_slice_rows(
    pool: &PgPool,
    owner_id: &AccountId,
    currency: &str,
    cutoff_start: DateTime<Utc>,
) -> Result<Vec<CategorySliceRow>, sqlx::Error> {
    let rows = sqlx::query_as!(
        CategorySliceRow,
        r#"
        SELECT c.name AS "category_name?",
               CAST(SUM(t.amount_minor) AS BIGINT) AS total_minor,
               COUNT(*) AS tx_count
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.owner_id = $1 AND t.currency = $2 AND t.direction = 'expense' AND t.occurred_at >= $3
        GROUP BY c.name
        ORDER BY total_minor DESC
        "#,
        owner_id.as_uuid(),
        currency,
        cutoff_start
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn recent_public_transactions(
    pool: &PgPool,
    owner_id: &AccountId,
    currency: &str,
    cutoff_start: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PublicTransaction>, sqlx::Error> {
    struct PublicTxRow {
        id: i64,
        direction: String,
        amount_minor: i64,
        currency: String,
        currency_exponent: i64,
        category_name: Option<String>,
        note: Option<String>,
        occurred_at: DateTime<Utc>,
    }

    let rows = sqlx::query_as!(
        PublicTxRow,
        r#"
        SELECT t.id, t.direction, t.amount_minor, t.currency, t.currency_exponent,
               c.name AS "category_name?", t.note, t.occurred_at
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.owner_id = $1 AND t.currency = $2 AND t.occurred_at >= $3
        ORDER BY t.occurred_at DESC
        LIMIT $4
        "#,
        owner_id.as_uuid(),
        currency,
        cutoff_start,
        limit
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| PublicTransaction {
            id: r.id,
            direction: r.direction,
            amount_minor: r.amount_minor,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            category_name: r.category_name,
            note: r.note,
            occurred_at: r.occurred_at,
        })
        .collect())
}

pub async fn create_share(
    pool: &PgPool,
    owner_id: &AccountId,
    token: &str,
    title: Option<&str>,
) -> Result<FinanceShare, sqlx::Error> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        INSERT INTO finance_shares (owner_id, token, is_enabled, title)
        VALUES ($1, $2, TRUE, $3)
        ON CONFLICT (owner_id)
        DO UPDATE SET token = $2, is_enabled = TRUE, title = $3, updated_at = NOW()
        RETURNING owner_id, token, is_enabled, title, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        token,
        title
    )
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

pub async fn upsert_share(
    pool: &PgPool,
    owner_id: &AccountId,
    token: &str,
    is_enabled: bool,
    title: Option<&str>,
) -> Result<FinanceShare, sqlx::Error> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        INSERT INTO finance_shares (owner_id, token, is_enabled, title)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (owner_id)
        DO UPDATE SET is_enabled = $3, title = $4, updated_at = NOW()
        RETURNING owner_id, token, is_enabled, title, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        token,
        is_enabled,
        title
    )
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

pub async fn get_share(
    pool: &PgPool,
    owner_id: &AccountId,
) -> Result<Option<FinanceShare>, sqlx::Error> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT owner_id, token, is_enabled, title, created_at, updated_at
        FROM finance_shares
        WHERE owner_id = $1
        "#,
        owner_id.as_uuid()
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_share_by_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<FinanceShare>, sqlx::Error> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        SELECT owner_id, token, is_enabled, title, created_at, updated_at
        FROM finance_shares
        WHERE token = $1
        "#,
        token
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn toggle_share(
    pool: &PgPool,
    owner_id: &AccountId,
    is_enabled: bool,
) -> Result<Option<FinanceShare>, sqlx::Error> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        UPDATE finance_shares
        SET is_enabled = $2, updated_at = NOW()
        WHERE owner_id = $1
        RETURNING owner_id, token, is_enabled, title, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        is_enabled
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn update_share_title(
    pool: &PgPool,
    owner_id: &AccountId,
    title: Option<&str>,
) -> Result<Option<FinanceShare>, sqlx::Error> {
    let row = sqlx::query_as!(
        ShareRow,
        r#"
        UPDATE finance_shares
        SET title = $2, updated_at = NOW()
        WHERE owner_id = $1
        RETURNING owner_id, token, is_enabled, title, created_at, updated_at
        "#,
        owner_id.as_uuid(),
        title
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}
