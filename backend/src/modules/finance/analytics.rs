use anyhow::Result;
use chrono::{DateTime, Datelike, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::modules::finance::money::exponent_for;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MonthlyCashflowBucket {
    pub month: String, // "YYYY-MM"
    pub income_minor: i64,
    pub expense_minor: i64,
    pub net_minor: i64,
    pub currency: String,
    pub currency_exponent: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ExpenseCategorySlice {
    pub category_name: String,
    pub amount_minor: i64,
    pub percentage: f64,
    pub tx_count: i64,
    pub currency: String,
    pub currency_exponent: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow, ToSchema)]
pub struct FinanceShare {
    pub owner_id: String,
    pub token: String,
    pub is_enabled: bool,
    pub title: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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

pub struct FinanceAnalyticsRepository;

impl FinanceAnalyticsRepository {
    /// Discovers the primary currency for an owner, or falls back to "USD".
    pub async fn primary_currency(pool: &SqlitePool, owner_id: &str) -> Result<String> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT currency FROM transactions WHERE owner_id = ? \
             GROUP BY currency ORDER BY COUNT(*) DESC LIMIT 1",
        )
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        Ok(row.unwrap_or_else(|| "USD".to_string()))
    }

    /// Fetches monthly cashflow history and expense category breakdown.
    pub async fn get_analytics_report(
        pool: &SqlitePool,
        owner_id: &str,
        currency_opt: Option<&str>,
        months_limit: i64,
    ) -> Result<FinanceAnalyticsReport> {
        let currency = match currency_opt {
            Some(c) if !c.trim().is_empty() => c.trim().to_uppercase(),
            _ => Self::primary_currency(pool, owner_id).await?,
        };
        let exponent = exponent_for(&currency) as i64;
        let months = months_limit.clamp(1, 36);

        let now = Utc::now();
        let current_year = now.year();
        let current_month = now.month();

        // Calculate sequence of YYYY-MM buckets in chronological order
        let mut month_keys: Vec<String> = Vec::with_capacity(months as usize);
        for i in (0..months).rev() {
            let total_months = (current_year as i32 * 12 + (current_month as i32 - 1)) - i as i32;
            let y = total_months / 12;
            let m = (total_months % 12) + 1;
            month_keys.push(format!("{y:04}-{m:02}"));
        }

        let earliest_month = month_keys
            .first()
            .cloned()
            .unwrap_or_else(|| "1970-01".into());
        let cutoff_start = format!("{earliest_month}-01 00:00:00");

        // Query transactions aggregated by month and direction
        let rows = sqlx::query_as::<_, (String, String, i64)>(
            r#"
            SELECT substr(occurred_at, 1, 7) AS month_str,
                   direction,
                   SUM(amount_minor) AS total_minor
            FROM transactions
            WHERE owner_id = ? AND currency = ? AND occurred_at >= ?
            GROUP BY month_str, direction
            ORDER BY month_str ASC
            "#,
        )
        .bind(owner_id)
        .bind(&currency)
        .bind(&cutoff_start)
        .fetch_all(pool)
        .await?;

        let mut monthly_cashflow: Vec<MonthlyCashflowBucket> = month_keys
            .iter()
            .map(|k| MonthlyCashflowBucket {
                month: k.clone(),
                income_minor: 0,
                expense_minor: 0,
                net_minor: 0,
                currency: currency.clone(),
                currency_exponent: exponent,
            })
            .collect();

        for (month_str, direction, sum_minor) in rows {
            if let Some(bucket) = monthly_cashflow.iter_mut().find(|b| b.month == month_str) {
                if direction == "expense" {
                    bucket.expense_minor += sum_minor;
                } else if direction == "income" {
                    bucket.income_minor += sum_minor;
                }
                bucket.net_minor = bucket.income_minor - bucket.expense_minor;
            }
        }

        let total_income: i64 = monthly_cashflow.iter().map(|b| b.income_minor).sum();
        let total_expense: i64 = monthly_cashflow.iter().map(|b| b.expense_minor).sum();
        let net_savings = total_income - total_expense;

        // Query expense category breakdown for the period
        let cat_rows = sqlx::query_as::<_, (Option<String>, i64, i64)>(
            r#"
            SELECT c.name,
                   COUNT(t.id) AS tx_cnt,
                   SUM(t.amount_minor) AS sum_minor
            FROM transactions t
            LEFT JOIN finance_categories c ON c.id = t.category_id
            WHERE t.owner_id = ? AND t.direction = 'expense' AND t.currency = ? AND t.occurred_at >= ?
            GROUP BY c.name
            ORDER BY sum_minor DESC
            "#,
        )
        .bind(owner_id)
        .bind(&currency)
        .bind(&cutoff_start)
        .fetch_all(pool)
        .await?;

        let mut expense_categories = Vec::new();
        for (cat_name, tx_cnt, sum_minor) in cat_rows {
            let percentage = if total_expense > 0 {
                ((sum_minor as f64 / total_expense as f64) * 1000.0).round() / 10.0
            } else {
                0.0
            };
            expense_categories.push(ExpenseCategorySlice {
                category_name: cat_name.unwrap_or_else(|| "Uncategorised".to_string()),
                amount_minor: sum_minor,
                percentage,
                tx_count: tx_cnt,
                currency: currency.clone(),
                currency_exponent: exponent,
            });
        }

        Ok(FinanceAnalyticsReport {
            currency,
            currency_exponent: exponent,
            months_requested: months,
            total_income_minor: total_income,
            total_expense_minor: total_expense,
            net_savings_minor: net_savings,
            monthly_cashflow,
            expense_categories,
        })
    }

    /// Fetches public share status for an owner.
    pub async fn get_share(pool: &SqlitePool, owner_id: &str) -> Result<Option<FinanceShare>> {
        let share = sqlx::query_as::<_, FinanceShare>(
            "SELECT owner_id, token, is_enabled, title, created_at, updated_at \
             FROM finance_shares WHERE owner_id = ?",
        )
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        Ok(share)
    }

    /// Creates or updates a public share record with a secure token.
    pub async fn upsert_share(
        pool: &SqlitePool,
        owner_id: &str,
        is_enabled: bool,
        title: Option<&str>,
    ) -> Result<FinanceShare> {
        let existing = Self::get_share(pool, owner_id).await?;
        let token = existing
            .map(|s| s.token)
            .unwrap_or_else(|| Uuid::new_v4().simple().to_string());

        sqlx::query(
            r#"
            INSERT INTO finance_shares (owner_id, token, is_enabled, title, updated_at)
            VALUES (?, ?, ?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT(owner_id) DO UPDATE SET
                is_enabled = excluded.is_enabled,
                title = excluded.title,
                updated_at = CURRENT_TIMESTAMP
            "#,
        )
        .bind(owner_id)
        .bind(&token)
        .bind(is_enabled)
        .bind(title)
        .execute(pool)
        .await?;

        let record = sqlx::query_as::<_, FinanceShare>(
            "SELECT owner_id, token, is_enabled, title, created_at, updated_at \
             FROM finance_shares WHERE owner_id = ?",
        )
        .bind(owner_id)
        .fetch_one(pool)
        .await?;

        Ok(record)
    }

    /// Disables public sharing for an owner.
    pub async fn disable_share(pool: &SqlitePool, owner_id: &str) -> Result<bool> {
        let res = sqlx::query(
            "UPDATE finance_shares SET is_enabled = 0, updated_at = CURRENT_TIMESTAMP WHERE owner_id = ?",
        )
        .bind(owner_id)
        .execute(pool)
        .await?;

        Ok(res.rows_affected() > 0)
    }

    /// Fetches public fiscal transparency report by token.
    pub async fn get_public_report(
        pool: &SqlitePool,
        token: &str,
        months: i64,
    ) -> Result<Option<PublicFinanceReport>> {
        let share = sqlx::query_as::<_, FinanceShare>(
            "SELECT owner_id, token, is_enabled, title, created_at, updated_at \
             FROM finance_shares WHERE token = ? AND is_enabled = 1",
        )
        .bind(token)
        .fetch_optional(pool)
        .await?;

        let share = match share {
            Some(s) => s,
            None => return Ok(None),
        };

        let report = Self::get_analytics_report(pool, &share.owner_id, None, months).await?;

        // Query sanitized recent transactions for public audit ledger (max 50)
        let tx_rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                i64,
                String,
                i64,
                Option<String>,
                Option<String>,
                DateTime<Utc>,
            ),
        >(
            r#"
            SELECT t.id, t.direction, t.amount_minor, t.currency, t.currency_exponent,
                   c.name AS category_name, t.note, t.occurred_at
            FROM transactions t
            LEFT JOIN finance_categories c ON c.id = t.category_id
            WHERE t.owner_id = ? AND t.currency = ?
            ORDER BY t.occurred_at DESC, t.id DESC
            LIMIT 50
            "#,
        )
        .bind(&share.owner_id)
        .bind(&report.currency)
        .fetch_all(pool)
        .await?;

        let recent_transactions = tx_rows
            .into_iter()
            .map(
                |(id, dir, amt, cur, exp, cat, note, occ)| PublicTransaction {
                    id,
                    direction: dir,
                    amount_minor: amt,
                    currency: cur,
                    currency_exponent: exp,
                    category_name: cat,
                    note,
                    occurred_at: occ,
                },
            )
            .collect();

        Ok(Some(PublicFinanceReport {
            title: share
                .title
                .unwrap_or_else(|| "Public Fiscal Transparency Report".to_string()),
            currency: report.currency,
            currency_exponent: report.currency_exponent,
            total_income_minor: report.total_income_minor,
            total_expense_minor: report.total_expense_minor,
            net_savings_minor: report.net_savings_minor,
            monthly_cashflow: report.monthly_cashflow,
            expense_categories: report.expense_categories,
            recent_transactions,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::modules::finance::repository::FinanceRepository;

    async fn setup_db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
    }

    #[tokio::test]
    async fn test_analytics_and_sharing_lifecycle() {
        let db = setup_db().await;
        let owner = "@alice:example.com";

        // Insert some transactions
        let now = Utc::now();
        FinanceRepository::create_transaction(
            &db.pool,
            "!room:x",
            owner,
            "Alice",
            "income",
            500_000,
            "USD",
            2,
            None,
            Some("Monthly Salary"),
            None,
            None,
            now,
        )
        .await
        .unwrap();

        let cat = FinanceRepository::create_category(&db.pool, owner, "Groceries")
            .await
            .unwrap()
            .unwrap();

        FinanceRepository::create_transaction(
            &db.pool,
            "!room:x",
            owner,
            "Alice",
            "expense",
            12_500,
            "USD",
            2,
            Some(cat.id),
            Some("Supermarket"),
            None,
            None,
            now,
        )
        .await
        .unwrap();

        // 1. Get analytics report
        let report =
            FinanceAnalyticsRepository::get_analytics_report(&db.pool, owner, Some("USD"), 6)
                .await
                .unwrap();

        assert_eq!(report.currency, "USD");
        assert_eq!(report.total_income_minor, 500_000);
        assert_eq!(report.total_expense_minor, 12_500);
        assert_eq!(report.net_savings_minor, 487_500);
        assert_eq!(report.monthly_cashflow.len(), 6);

        let cur_month = report.monthly_cashflow.last().unwrap();
        assert_eq!(cur_month.income_minor, 500_000);
        assert_eq!(cur_month.expense_minor, 12_500);

        assert_eq!(report.expense_categories.len(), 1);
        assert_eq!(report.expense_categories[0].category_name, "Groceries");
        assert_eq!(report.expense_categories[0].percentage, 100.0);

        // 2. Share lifecycle
        let share = FinanceAnalyticsRepository::upsert_share(
            &db.pool,
            owner,
            true,
            Some("Alice's Public Budget"),
        )
        .await
        .unwrap();
        assert!(share.is_enabled);
        assert_eq!(share.title.as_deref(), Some("Alice's Public Budget"));

        // Fetch public report by token
        let pub_report = FinanceAnalyticsRepository::get_public_report(&db.pool, &share.token, 6)
            .await
            .unwrap()
            .expect("public report should exist");
        assert_eq!(pub_report.title, "Alice's Public Budget");
        assert_eq!(pub_report.recent_transactions.len(), 2);

        // Disable share
        FinanceAnalyticsRepository::disable_share(&db.pool, owner)
            .await
            .unwrap();
        let disabled = FinanceAnalyticsRepository::get_public_report(&db.pool, &share.token, 6)
            .await
            .unwrap();
        assert!(disabled.is_none());
    }
}
