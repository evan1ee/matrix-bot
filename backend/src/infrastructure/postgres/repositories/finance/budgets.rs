//! Monthly budgets and their once-per-month alert markers.
//!
//! Every read joins `finance_categories` for the scope name. Rows are converted
//! to the domain [`Budget`] at the edge.

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;
use uuid::Uuid;

use crate::domain::accounts::AccountId;
use crate::domain::finance::budget::{
    Budget, BudgetPatch, BudgetScope, Month, NewBudget, Threshold,
};

#[derive(Debug)]
struct BudgetRow {
    id: i64,
    owner_id: Uuid,
    category_id: Option<i64>,
    category_name: Option<String>,
    currency: String,
    currency_exponent: i64,
    limit_minor: i64,
    alerts_enabled: bool,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<BudgetRow> for Budget {
    fn from(r: BudgetRow) -> Self {
        let scope = match (r.category_id, r.category_name) {
            (Some(id), Some(name)) => BudgetScope::Category { id, name },
            _ => BudgetScope::Overall,
        };
        Self {
            id: r.id,
            owner_id: AccountId::from_uuid(r.owner_id),
            scope,
            currency: r.currency,
            currency_exponent: r.currency_exponent,
            limit_minor: r.limit_minor,
            alerts_enabled: r.alerts_enabled,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

/// Expense total for one (category, currency) bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpendRow {
    pub category_id: Option<i64>,
    pub currency: String,
    pub spent_minor: i64,
}

/// A duplicate (owner, scope, currency) surfaces as a unique violation.
pub async fn insert(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    category_id: Option<i64>,
    b: &NewBudget,
) -> sqlx::Result<Budget> {
    let row = sqlx::query_as!(
        BudgetRow,
        r#"
        WITH b AS (
            INSERT INTO finance_budgets
                (owner_id, category_id, currency, currency_exponent, limit_minor, alerts_enabled)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING *
        )
        SELECT b.id AS "id!", b.owner_id AS "owner_id!", b.category_id,
               c.name AS "category_name?", b.currency AS "currency!",
               b.currency_exponent AS "currency_exponent!", b.limit_minor AS "limit_minor!",
               b.alerts_enabled AS "alerts_enabled!", b.created_at AS "created_at!",
               b.updated_at AS "updated_at!"
        FROM b LEFT JOIN finance_categories c ON c.id = b.category_id
        "#,
        owner.as_uuid(),
        category_id,
        b.currency(),
        b.currency_exponent(),
        b.limit_minor(),
        b.alerts_enabled().unwrap_or(true)
    )
    .fetch_one(ex)
    .await?;
    Ok(row.into())
}

pub async fn find_owned(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    id: i64,
) -> sqlx::Result<Option<Budget>> {
    let row = sqlx::query_as!(
        BudgetRow,
        r#"
        SELECT b.id, b.owner_id, b.category_id, c.name AS "category_name?", b.currency,
               b.currency_exponent, b.limit_minor, b.alerts_enabled, b.created_at, b.updated_at
        FROM finance_budgets b LEFT JOIN finance_categories c ON c.id = b.category_id
        WHERE b.owner_id = $1 AND b.id = $2
        "#,
        owner.as_uuid(),
        id
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.map(Into::into))
}

/// Locks the budget for one (scope, currency); `None` category means overall.
pub async fn find_by_scope_for_update(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    category_id: Option<i64>,
    currency: &str,
) -> sqlx::Result<Option<Budget>> {
    let row = sqlx::query_as!(
        BudgetRow,
        r#"
        SELECT b.id, b.owner_id, b.category_id, c.name AS "category_name?", b.currency,
               b.currency_exponent, b.limit_minor, b.alerts_enabled, b.created_at, b.updated_at
        FROM finance_budgets b LEFT JOIN finance_categories c ON c.id = b.category_id
        WHERE b.owner_id = $1 AND b.category_id IS NOT DISTINCT FROM $2 AND b.currency = $3
        FOR UPDATE OF b
        "#,
        owner.as_uuid(),
        category_id,
        currency
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.map(Into::into))
}

/// Overall budgets first, then by category name, then currency.
pub async fn list(ex: impl PgExecutor<'_>, owner: &AccountId) -> sqlx::Result<Vec<Budget>> {
    let rows = sqlx::query_as!(
        BudgetRow,
        r#"
        SELECT b.id, b.owner_id, b.category_id, c.name AS "category_name?", b.currency,
               b.currency_exponent, b.limit_minor, b.alerts_enabled, b.created_at, b.updated_at
        FROM finance_budgets b LEFT JOIN finance_categories c ON c.id = b.category_id
        WHERE b.owner_id = $1
        ORDER BY (b.category_id IS NOT NULL), LOWER(c.name), b.currency, b.id
        "#,
        owner.as_uuid()
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Every currency's budget for one scope (`None` = overall).
pub async fn list_for_scope(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    category_id: Option<i64>,
) -> sqlx::Result<Vec<Budget>> {
    let rows = sqlx::query_as!(
        BudgetRow,
        r#"
        SELECT b.id, b.owner_id, b.category_id, c.name AS "category_name?", b.currency,
               b.currency_exponent, b.limit_minor, b.alerts_enabled, b.created_at, b.updated_at
        FROM finance_budgets b LEFT JOIN finance_categories c ON c.id = b.category_id
        WHERE b.owner_id = $1 AND b.category_id IS NOT DISTINCT FROM $2
        ORDER BY b.currency, b.id
        "#,
        owner.as_uuid(),
        category_id
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

/// Locks the alert-enabled budgets an expense in `currency` / `category_id`
/// counts toward (the overall one and that category's), in id order. Locking
/// before the month's spend is summed makes concurrent expenses evaluate one
/// after the other, so neither misses a threshold crossing.
pub async fn lock_matching(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    currency: &str,
    category_id: Option<i64>,
) -> sqlx::Result<Vec<Budget>> {
    let rows = sqlx::query_as!(
        BudgetRow,
        r#"
        SELECT b.id, b.owner_id, b.category_id, c.name AS "category_name?", b.currency,
               b.currency_exponent, b.limit_minor, b.alerts_enabled, b.created_at, b.updated_at
        FROM finance_budgets b LEFT JOIN finance_categories c ON c.id = b.category_id
        WHERE b.owner_id = $1 AND b.alerts_enabled AND b.currency = $2
          AND (b.category_id IS NULL OR b.category_id = $3)
        ORDER BY b.id
        FOR UPDATE OF b
        "#,
        owner.as_uuid(),
        currency,
        category_id
    )
    .fetch_all(ex)
    .await?;
    Ok(rows.into_iter().map(Into::into).collect())
}

pub async fn update(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    id: i64,
    patch: &BudgetPatch,
) -> sqlx::Result<Option<Budget>> {
    let row = sqlx::query_as!(
        BudgetRow,
        r#"
        WITH b AS (
            UPDATE finance_budgets
            SET limit_minor = COALESCE($3, limit_minor),
                alerts_enabled = COALESCE($4, alerts_enabled),
                updated_at = now()
            WHERE owner_id = $1 AND id = $2
            RETURNING *
        )
        SELECT b.id AS "id!", b.owner_id AS "owner_id!", b.category_id,
               c.name AS "category_name?", b.currency AS "currency!",
               b.currency_exponent AS "currency_exponent!", b.limit_minor AS "limit_minor!",
               b.alerts_enabled AS "alerts_enabled!", b.created_at AS "created_at!",
               b.updated_at AS "updated_at!"
        FROM b LEFT JOIN finance_categories c ON c.id = b.category_id
        "#,
        owner.as_uuid(),
        id,
        patch.limit_minor,
        patch.alerts_enabled
    )
    .fetch_optional(ex)
    .await?;
    Ok(row.map(Into::into))
}

pub async fn delete(ex: impl PgExecutor<'_>, owner: &AccountId, id: i64) -> sqlx::Result<bool> {
    let done = sqlx::query!(
        "DELETE FROM finance_budgets WHERE owner_id = $1 AND id = $2",
        owner.as_uuid(),
        id
    )
    .execute(ex)
    .await?;
    Ok(done.rows_affected() > 0)
}

/// Expense totals in `[from, to)` per (category, currency), optionally for one
/// currency. Any `source_kind` counts; uncategorised rows have `category_id: None`.
pub async fn month_spend(
    ex: impl PgExecutor<'_>,
    owner: &AccountId,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    currency: Option<&str>,
) -> sqlx::Result<Vec<SpendRow>> {
    sqlx::query_as!(
        SpendRow,
        r#"
        SELECT category_id, currency AS "currency!", SUM(amount_minor)::BIGINT AS "spent_minor!"
        FROM transactions
        WHERE owner_id = $1 AND direction = 'expense'
          AND occurred_at >= $2 AND occurred_at < $3
          AND ($4::TEXT IS NULL OR currency = $4)
        GROUP BY category_id, currency
        ORDER BY category_id NULLS FIRST, currency
        "#,
        owner.as_uuid(),
        from,
        to,
        currency
    )
    .fetch_all(ex)
    .await
}

/// Marks a threshold as alerted for a month; `true` only for the first caller.
pub async fn record_alert(
    ex: impl PgExecutor<'_>,
    budget_id: i64,
    month: Month,
    threshold: Threshold,
) -> sqlx::Result<bool> {
    let inserted = sqlx::query_scalar!(
        r#"
        INSERT INTO finance_budget_alerts (budget_id, month_start, threshold)
        VALUES ($1, $2, $3)
        ON CONFLICT DO NOTHING
        RETURNING budget_id
        "#,
        budget_id,
        month.first_day(),
        threshold.pct()
    )
    .fetch_optional(ex)
    .await?;
    Ok(inserted.is_some())
}

/// Test-only: how many threshold alerts have been recorded in this schema.
#[cfg(test)]
pub async fn count_alerts(ex: impl PgExecutor<'_>) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM finance_budget_alerts")
        .fetch_one(ex)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};
    use sqlx::PgPool;

    async fn setup() -> (PgPool, AccountId) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        let owner = AccountId::generate();
        crate::db::pool::seed_account(&pool, &owner).await;
        (pool, owner)
    }

    async fn category(pool: &PgPool, owner: &AccountId, name: &str) -> i64 {
        sqlx::query_scalar!(
            "INSERT INTO finance_categories (owner_id, name) VALUES ($1, $2) RETURNING id",
            owner.as_uuid(),
            name
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    fn new_budget(category: Option<&str>, currency: &str, limit: i64) -> NewBudget {
        NewBudget::new(category, currency, limit, Some(true)).unwrap()
    }

    #[allow(clippy::too_many_arguments)] // test fixture: one positional field per column
    async fn tx_row(
        pool: &PgPool,
        owner: &AccountId,
        direction: &str,
        amount: i64,
        currency: &str,
        category_id: Option<i64>,
        source_kind: Option<&str>,
        at: DateTime<Utc>,
    ) {
        sqlx::query!(
            "INSERT INTO transactions (room_id, owner_id, sender_name, direction, amount_minor, \
             currency, currency_exponent, category_id, source_kind, occurred_at) \
             VALUES ('r', $1, 's', $2, $3, $4, 2, $5, $6, $7)",
            owner.as_uuid(),
            direction,
            amount,
            currency,
            category_id,
            source_kind,
            at
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn insert_and_find_round_trip() {
        let (pool, owner) = setup().await;
        let food = category(&pool, &owner, "Food").await;

        let overall = insert(&pool, &owner, None, &new_budget(None, "eur", 100_000))
            .await
            .unwrap();
        assert_eq!(overall.scope, BudgetScope::Overall);
        assert_eq!(overall.currency, "EUR");
        assert_eq!(overall.limit_minor, 100_000);
        assert_eq!(overall.owner_id, owner);

        let cat = insert(
            &pool,
            &owner,
            Some(food),
            &new_budget(Some("Food"), "EUR", 30_000),
        )
        .await
        .unwrap();
        assert_eq!(
            cat.scope,
            BudgetScope::Category {
                id: food,
                name: "Food".into()
            }
        );
        assert!(cat.alerts_enabled);

        assert_eq!(
            find_owned(&pool, &owner, cat.id).await.unwrap(),
            Some(cat.clone())
        );
        let listed = list(&pool, &owner).await.unwrap();
        assert_eq!(
            listed.iter().map(|b| b.id).collect::<Vec<_>>(),
            vec![overall.id, cat.id]
        );
        let locked = find_by_scope_for_update(&pool, &owner, None, "EUR")
            .await
            .unwrap();
        assert_eq!(locked.map(|b| b.id), Some(overall.id));
        assert!(find_by_scope_for_update(&pool, &owner, None, "USD")
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            list_for_scope(&pool, &owner, Some(food))
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn second_overall_budget_same_currency_is_unique_violation() {
        let (pool, owner) = setup().await;
        insert(&pool, &owner, None, &new_budget(None, "EUR", 100))
            .await
            .unwrap();
        let err = insert(&pool, &owner, None, &new_budget(None, "EUR", 200))
            .await
            .unwrap_err();
        assert!(matches!(&err, sqlx::Error::Database(e) if e.is_unique_violation()));
        insert(&pool, &owner, None, &new_budget(None, "USD", 200))
            .await
            .expect("another currency is a different budget");
    }

    #[tokio::test]
    async fn budgets_are_owner_isolated() {
        let (pool, owner) = setup().await;
        let other = AccountId::generate();
        crate::db::pool::seed_account(&pool, &other).await;
        let b = insert(&pool, &owner, None, &new_budget(None, "EUR", 100))
            .await
            .unwrap();

        assert!(find_owned(&pool, &other, b.id).await.unwrap().is_none());
        let patch = BudgetPatch {
            limit_minor: Some(5),
            alerts_enabled: None,
        };
        assert!(update(&pool, &other, b.id, &patch).await.unwrap().is_none());
        assert!(!delete(&pool, &other, b.id).await.unwrap());
        assert!(list(&pool, &other).await.unwrap().is_empty());

        let updated = update(&pool, &owner, b.id, &patch).await.unwrap().unwrap();
        assert_eq!((updated.limit_minor, updated.alerts_enabled), (5, true));
        assert!(updated.updated_at >= b.updated_at);
        assert!(delete(&pool, &owner, b.id).await.unwrap());
    }

    #[tokio::test]
    async fn month_spend_counts_only_expenses_in_bounds() {
        let (pool, owner) = setup().await;
        let food = category(&pool, &owner, "Food").await;
        let from = Utc.with_ymd_and_hms(2026, 10, 1, 0, 0, 0).unwrap();
        let to = Utc.with_ymd_and_hms(2026, 11, 1, 0, 0, 0).unwrap();
        let mid = from + Duration::days(10);
        let before = from - Duration::microseconds(1);

        tx_row(
            &pool,
            &owner,
            "expense",
            1_000,
            "EUR",
            Some(food),
            None,
            from,
        )
        .await;
        tx_row(&pool, &owner, "expense", 200, "EUR", Some(food), None, mid).await;
        let bill = Some("recurring_bill");
        tx_row(&pool, &owner, "expense", 300, "EUR", None, bill, mid).await;
        tx_row(&pool, &owner, "income", 9_999, "EUR", Some(food), None, mid).await;
        tx_row(
            &pool,
            &owner,
            "expense",
            9_999,
            "USD",
            Some(food),
            None,
            mid,
        )
        .await;
        tx_row(&pool, &owner, "expense", 9_999, "EUR", Some(food), None, to).await;
        tx_row(
            &pool,
            &owner,
            "expense",
            9_999,
            "EUR",
            Some(food),
            None,
            before,
        )
        .await;

        let rows = month_spend(&pool, &owner, from, to, Some("EUR"))
            .await
            .unwrap();
        assert_eq!(
            rows,
            vec![
                SpendRow {
                    category_id: None,
                    currency: "EUR".into(),
                    spent_minor: 300
                },
                SpendRow {
                    category_id: Some(food),
                    currency: "EUR".into(),
                    spent_minor: 1_200
                },
            ]
        );
        let all = month_spend(&pool, &owner, from, to, None).await.unwrap();
        assert!(all
            .iter()
            .any(|r| r.currency == "USD" && r.spent_minor == 9_999));
    }

    #[tokio::test]
    async fn deleting_category_cascades_its_budgets() {
        let (pool, owner) = setup().await;
        let food = category(&pool, &owner, "Food").await;
        let b = insert(
            &pool,
            &owner,
            Some(food),
            &new_budget(Some("Food"), "EUR", 100),
        )
        .await
        .unwrap();
        let overall = insert(&pool, &owner, None, &new_budget(None, "EUR", 100))
            .await
            .unwrap();
        sqlx::query!("DELETE FROM finance_categories WHERE id = $1", food)
            .execute(&pool)
            .await
            .unwrap();
        assert!(find_owned(&pool, &owner, b.id).await.unwrap().is_none());
        assert!(find_owned(&pool, &owner, overall.id)
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn record_alert_is_idempotent() {
        let (pool, owner) = setup().await;
        let b = insert(&pool, &owner, None, &new_budget(None, "EUR", 100))
            .await
            .unwrap();
        let month = Month::parse("2026-10").unwrap();
        assert!(record_alert(&pool, b.id, month, Threshold::Warning)
            .await
            .unwrap());
        assert!(!record_alert(&pool, b.id, month, Threshold::Warning)
            .await
            .unwrap());
        assert!(record_alert(&pool, b.id, month, Threshold::Exceeded)
            .await
            .unwrap());
        let next = Month::parse("2026-11").unwrap();
        assert!(record_alert(&pool, b.id, next, Threshold::Warning)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn lock_matching_returns_overall_and_category_only() {
        let (pool, owner) = setup().await;
        let food = category(&pool, &owner, "Food").await;
        let travel = category(&pool, &owner, "Travel").await;
        let overall = insert(&pool, &owner, None, &new_budget(None, "EUR", 100))
            .await
            .unwrap();
        let cat = insert(
            &pool,
            &owner,
            Some(food),
            &new_budget(Some("Food"), "EUR", 100),
        )
        .await
        .unwrap();
        let other_cat = new_budget(Some("Travel"), "EUR", 100);
        insert(&pool, &owner, Some(travel), &other_cat)
            .await
            .unwrap();
        insert(&pool, &owner, None, &new_budget(None, "USD", 100))
            .await
            .unwrap();
        let muted = NewBudget::new(Some("Food"), "GBP", 100, Some(false)).unwrap();
        insert(&pool, &owner, Some(food), &muted).await.unwrap();
        let off = NewBudget::new(None, "JPY", 100, Some(false)).unwrap();
        insert(&pool, &owner, None, &off).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        let got = lock_matching(&mut *tx, &owner, "EUR", Some(food))
            .await
            .unwrap();
        assert_eq!(
            got.iter().map(|b| b.id).collect::<Vec<_>>(),
            vec![overall.id, cat.id]
        );
        let only_overall = lock_matching(&mut *tx, &owner, "EUR", None).await.unwrap();
        assert_eq!(
            only_overall.iter().map(|b| b.id).collect::<Vec<_>>(),
            vec![overall.id]
        );
        let gbp = lock_matching(&mut *tx, &owner, "GBP", Some(food))
            .await
            .unwrap();
        assert!(gbp.is_empty());
        let jpy = lock_matching(&mut *tx, &owner, "JPY", None).await.unwrap();
        assert!(jpy.is_empty());
        tx.rollback().await.unwrap();
    }
}
