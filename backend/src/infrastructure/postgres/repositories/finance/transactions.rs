use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::AccountId;
use crate::domain::finance::{
    CategoryTotal, CurrencySummary, Direction, ListTransactionsFilter, NewTransaction, Transaction,
};

#[derive(Debug, sqlx::FromRow)]
struct TransactionRow {
    id: i64,
    room_id: String,
    owner_id: String,
    sender_name: String,
    direction: String,
    amount_minor: i64,
    currency: String,
    currency_exponent: i64,
    category_id: Option<i64>,
    category_name: Option<String>,
    note: Option<String>,
    source_kind: Option<String>,
    source_id: Option<i64>,
    occurred_at: DateTime<Utc>,
    created_at: Option<DateTime<Utc>>,
}

impl From<TransactionRow> for Transaction {
    fn from(r: TransactionRow) -> Self {
        Self {
            id: r.id,
            room_id: r.room_id,
            owner_id: AccountId::new(r.owner_id),
            sender_name: r.sender_name,
            direction: Direction::parse(&r.direction).unwrap_or(Direction::Expense),
            amount_minor: r.amount_minor,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            category_id: r.category_id,
            category_name: r.category_name,
            note: r.note,
            source_kind: r.source_kind,
            source_id: r.source_id,
            occurred_at: r.occurred_at,
            created_at: r.created_at,
        }
    }
}

pub async fn create_transaction(
    pool: &PgPool,
    owner_id: &AccountId,
    input: &NewTransaction<'_>,
    category_id: Option<i64>,
) -> Result<Transaction, sqlx::Error> {
    let (source_kind, source_id) = match input.source {
        Some((k, id)) => (Some(k), Some(id)),
        None => (None, None),
    };

    let id = sqlx::query_scalar!(
        r#"
        INSERT INTO transactions
            (room_id, owner_id, sender_name, direction, amount_minor, currency,
             currency_exponent, category_id, note, source_kind, source_id, occurred_at)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        RETURNING id
        "#,
        input.room_id,
        owner_id.as_str(),
        input.sender_name,
        input.direction.as_str(),
        input.amount_minor,
        input.currency,
        input.currency_exponent,
        category_id,
        input.note,
        source_kind,
        source_id,
        input.occurred_at
    )
    .fetch_one(pool)
    .await?;

    let row = sqlx::query_as!(
        TransactionRow,
        r#"
        SELECT t.id, t.room_id, t.owner_id, t.sender_name, t.direction,
               t.amount_minor, t.currency, t.currency_exponent, t.category_id,
               c.name AS "category_name?", t.note, t.source_kind, t.source_id,
               t.occurred_at, t.created_at
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.id = $1
        "#,
        id
    )
    .fetch_one(pool)
    .await?;

    Ok(row.into())
}

pub async fn find(pool: &PgPool, id: i64) -> Result<Option<Transaction>, sqlx::Error> {
    let row = sqlx::query_as!(
        TransactionRow,
        r#"
        SELECT t.id, t.room_id, t.owner_id, t.sender_name, t.direction,
               t.amount_minor, t.currency, t.currency_exponent, t.category_id,
               c.name AS "category_name?", t.note, t.source_kind, t.source_id,
               t.occurred_at, t.created_at
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.id = $1
        "#,
        id
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

pub async fn find_owned(
    pool: &PgPool,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<Transaction>, sqlx::Error> {
    let row = sqlx::query_as!(
        TransactionRow,
        r#"
        SELECT t.id, t.room_id, t.owner_id, t.sender_name, t.direction,
               t.amount_minor, t.currency, t.currency_exponent, t.category_id,
               c.name AS "category_name?", t.note, t.source_kind, t.source_id,
               t.occurred_at, t.created_at
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.id = $1 AND t.owner_id = $2
        "#,
        id,
        owner_id.as_str()
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(Into::into))
}

#[allow(clippy::too_many_arguments)]
pub async fn update(
    pool: &PgPool,
    owner_id: &AccountId,
    id: i64,
    direction: Direction,
    amount_minor: i64,
    currency: &str,
    currency_exponent: i64,
    category_id: Option<i64>,
    note: Option<&str>,
    occurred_at: DateTime<Utc>,
) -> Result<Option<Transaction>, sqlx::Error> {
    let rows_affected = sqlx::query!(
        r#"
        UPDATE transactions
        SET direction = $1, amount_minor = $2, currency = $3, currency_exponent = $4,
            category_id = $5, note = $6, occurred_at = $7
        WHERE id = $8 AND owner_id = $9
        "#,
        direction.as_str(),
        amount_minor,
        currency,
        currency_exponent,
        category_id,
        note,
        occurred_at,
        id,
        owner_id.as_str()
    )
    .execute(pool)
    .await?
    .rows_affected();

    if rows_affected == 0 {
        return Ok(None);
    }

    find_owned(pool, owner_id, id).await
}

pub async fn delete(
    pool: &PgPool,
    owner_id: &AccountId,
    id: i64,
) -> Result<Option<Transaction>, sqlx::Error> {
    let existing = find_owned(pool, owner_id, id).await?;
    if existing.is_none() {
        return Ok(None);
    }

    sqlx::query!(
        "DELETE FROM transactions WHERE id = $1 AND owner_id = $2",
        id,
        owner_id.as_str()
    )
    .execute(pool)
    .await?;

    Ok(existing)
}

pub async fn list(
    pool: &PgPool,
    owner_id: &AccountId,
    filter: &ListTransactionsFilter<'_>,
) -> Result<Vec<Transaction>, sqlx::Error> {
    let direction_str = filter.direction.map(|d| d.as_str());
    let limit = filter.limit.unwrap_or(25);
    let offset = filter.offset.unwrap_or(0);

    let rows = sqlx::query_as!(
        TransactionRow,
        r#"
        SELECT t.id, t.room_id, t.owner_id, t.sender_name, t.direction,
               t.amount_minor, t.currency, t.currency_exponent, t.category_id,
               c.name AS "category_name?", t.note, t.source_kind, t.source_id,
               t.occurred_at, t.created_at
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.owner_id = $1
          AND ($2::text IS NULL OR t.direction = $2)
          AND ($3::bigint IS NULL OR t.category_id = $3)
          AND ($4::timestamptz IS NULL OR t.occurred_at >= $4)
          AND ($5::timestamptz IS NULL OR t.occurred_at <= $5)
          AND ($6::text IS NULL OR t.currency = $6)
        ORDER BY t.occurred_at DESC, t.id DESC
        LIMIT $7 OFFSET $8
        "#,
        owner_id.as_str(),
        direction_str,
        filter.category_id,
        filter.since,
        filter.until,
        filter.currency,
        limit,
        offset
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn count(
    pool: &PgPool,
    owner_id: &AccountId,
    direction: Option<Direction>,
) -> Result<i64, sqlx::Error> {
    let dir_str = direction.map(|d| d.as_str());
    let count = sqlx::query_scalar!(
        r#"
        SELECT COUNT(*)
        FROM transactions
        WHERE owner_id = $1 AND ($2::text IS NULL OR direction = $2)
        "#,
        owner_id.as_str(),
        dir_str
    )
    .fetch_one(pool)
    .await?;

    Ok(count.unwrap_or(0))
}

pub async fn summary(
    pool: &PgPool,
    owner_id: &AccountId,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<CurrencySummary>, sqlx::Error> {
    struct SummaryRow {
        currency: String,
        currency_exponent: i64,
        category: Option<String>,
        direction: String,
        total: Option<i64>,
    }

    let rows = sqlx::query_as!(
        SummaryRow,
        r#"
        SELECT t.currency, t.currency_exponent, c.name AS "category?", t.direction,
               CAST(SUM(t.amount_minor) AS BIGINT) AS total
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.owner_id = $1 AND t.occurred_at >= $2 AND t.occurred_at < $3
        GROUP BY t.currency, t.currency_exponent, c.name, t.direction
        ORDER BY t.currency ASC
        "#,
        owner_id.as_str(),
        from,
        to
    )
    .fetch_all(pool)
    .await?;

    let mut summaries: Vec<CurrencySummary> = Vec::new();

    for row in rows {
        let total = row.total.unwrap_or(0);
        let entry = match summaries.iter_mut().find(|s| s.currency == row.currency) {
            Some(existing) => existing,
            None => {
                summaries.push(CurrencySummary {
                    currency: row.currency.clone(),
                    currency_exponent: row.currency_exponent,
                    expense_minor: 0,
                    income_minor: 0,
                    net_minor: 0,
                    categories: Vec::new(),
                });
                summaries.last_mut().expect("just pushed")
            }
        };

        let is_expense = row.direction == "expense";
        if is_expense {
            entry.expense_minor += total;
        } else {
            entry.income_minor += total;
        }

        match entry
            .categories
            .iter_mut()
            .find(|c| c.category == row.category)
        {
            Some(bucket) => {
                if is_expense {
                    bucket.expense_minor += total;
                } else {
                    bucket.income_minor += total;
                }
            }
            None => entry.categories.push(CategoryTotal {
                category: row.category,
                expense_minor: if is_expense { total } else { 0 },
                income_minor: if is_expense { 0 } else { total },
            }),
        }
    }

    for entry in &mut summaries {
        entry.net_minor = entry.income_minor - entry.expense_minor;
        entry
            .categories
            .sort_by_key(|c| std::cmp::Reverse(c.expense_minor));
    }

    Ok(summaries)
}

pub async fn get_category_summary(
    pool: &PgPool,
    owner_id: &AccountId,
    category_id: Option<i64>,
    currency: &str,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
) -> Result<Option<CategoryTotal>, sqlx::Error> {
    struct CatSumRow {
        category_name: Option<String>,
        total: Option<i64>,
    }

    let row = sqlx::query_as!(
        CatSumRow,
        r#"
        SELECT c.name AS "category_name?", CAST(SUM(t.amount_minor) AS BIGINT) AS total
        FROM transactions t
        LEFT JOIN finance_categories c ON c.id = t.category_id
        WHERE t.owner_id = $1
          AND t.currency = $2
          AND t.direction = 'expense'
          AND ($3::bigint IS NULL OR t.category_id = $3)
          AND ($4::timestamptz IS NULL OR t.occurred_at >= $4)
          AND ($5::timestamptz IS NULL OR t.occurred_at <= $5)
        GROUP BY c.name
        "#,
        owner_id.as_str(),
        currency,
        category_id,
        since,
        until
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| CategoryTotal {
        category: r.category_name,
        expense_minor: r.total.unwrap_or(0),
        income_minor: 0,
    }))
}
