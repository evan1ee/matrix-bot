use crate::db::DbPool;
use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::modules::finance::models::{Category, CategoryTotal, CurrencySummary, Transaction};

/// Ledger columns, joined to the category name so listing is one query rather
/// than N+1.
const TX_COLUMNS: &str = "t.id, t.room_id, t.owner_id, t.sender_name, t.direction, \
                          t.amount_minor, t.currency, t.currency_exponent, t.category_id, \
                          c.name AS category_name, t.note, t.source_kind, t.source_id, \
                          t.occurred_at, t.created_at";

pub struct FinanceRepository;

impl FinanceRepository {
    // ---- categories ------------------------------------------------------

    /// Finds an existing category by name or creates a new one.
    pub async fn find_or_create_category(
        pool: &DbPool,
        owner_id: &str,
        name: &str,
    ) -> Result<Option<Category>> {
        if let Some(cat) = Self::find_category_by_name(pool, owner_id, name).await? {
            return Ok(Some(cat));
        }
        Self::create_category(pool, owner_id, name).await
    }

    /// Creates a category. Returns `None` when the owner already has one by
    /// that name — the unique index is what enforces it, so two concurrent
    /// writers cannot both win.
    pub async fn create_category(
        pool: &DbPool,
        owner_id: &str,
        name: &str,
    ) -> Result<Option<Category>> {
        let result = crate::db_scalar_one!(
            pool,
            i64,
            "INSERT INTO finance_categories (owner_id, name) VALUES (?, ?) RETURNING id",
            owner_id,
            name
        );

        let id = match result {
            Ok(id) => id,
            Err(sqlx::Error::Database(err)) if err.is_unique_violation() => return Ok(None),
            Err(err) => return Err(err.into()),
        };

        let category = crate::db_fetch_one!(
            pool,
            Category,
            "SELECT id, owner_id, name, created_at FROM finance_categories WHERE id = ?",
            id
        )?;

        Ok(Some(category))
    }

    pub async fn list_categories(pool: &DbPool, owner_id: &str) -> Result<Vec<Category>> {
        let categories = crate::db_fetch_all!(
            pool,
            Category,
            "SELECT id, owner_id, name, created_at FROM finance_categories \
             WHERE owner_id = ? ORDER BY name COLLATE NOCASE ASC",
            owner_id
        )?;

        Ok(categories)
    }

    pub async fn find_category_by_name(
        pool: &DbPool,
        owner_id: &str,
        name: &str,
    ) -> Result<Option<Category>> {
        let category = crate::db_fetch_one!(
            pool,
            Category,
            "SELECT id, owner_id, name, created_at FROM finance_categories \
             WHERE owner_id = ? AND name = ? COLLATE NOCASE",
            owner_id,
            name
        )
        .ok();

        Ok(category)
    }

    /// Deletes a category and un-categorises its transactions.
    ///
    /// Financial records are never destroyed as a side effect of tidying up the
    /// category list. Returns how many rows were left uncategorised so the
    /// caller can say so.
    pub async fn delete_category(pool: &DbPool, owner_id: &str, id: i64) -> Result<Option<u64>> {
        let owns: Option<(i64,)> = crate::db_fetch_optional!(
            pool,
            _,
            "SELECT id FROM finance_categories WHERE id = ? AND owner_id = ?",
            id,
            owner_id
        )?;

        if owns.is_none() {
            return Ok(None);
        }

        let orphaned = crate::db_execute!(
            pool,
            "UPDATE transactions SET category_id = NULL WHERE category_id = ? AND owner_id = ?",
            id,
            owner_id
        )?;

        crate::db_execute!(
            pool,
            "DELETE FROM finance_categories WHERE id = ? AND owner_id = ?",
            id,
            owner_id
        )?;

        Ok(Some(orphaned))
    }

    // ---- transactions ----------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub async fn create_transaction(
        pool: &DbPool,
        room_id: &str,
        owner_id: &str,
        sender_name: &str,
        direction: &str,
        amount_minor: i64,
        currency: &str,
        currency_exponent: i64,
        category_id: Option<i64>,
        note: Option<&str>,
        source_kind: Option<&str>,
        source_id: Option<i64>,
        occurred_at: DateTime<Utc>,
    ) -> Result<Transaction> {
        let res = crate::db_scalar_one!(
            pool,
            i64,
            r#"
            INSERT INTO transactions
                (room_id, owner_id, sender_name, direction, amount_minor, currency,
                 currency_exponent, category_id, note, source_kind, source_id, occurred_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            RETURNING id
            "#,
            room_id,
            owner_id,
            sender_name,
            direction,
            amount_minor,
            currency,
            currency_exponent,
            category_id,
            note,
            source_kind,
            source_id,
            occurred_at
        )?;

        Self::find(pool, res)
            .await?
            .ok_or_else(|| anyhow::anyhow!("inserted transaction vanished before it could be read"))
    }

    pub async fn find(pool: &DbPool, id: i64) -> Result<Option<Transaction>> {
        let tx = crate::db_fetch_optional!(
            pool,
            Transaction,
            &format!(
                "SELECT {TX_COLUMNS} FROM transactions t \
             LEFT JOIN finance_categories c ON c.id = t.category_id WHERE t.id = ?"
            ),
            id
        )?;

        Ok(tx)
    }

    /// Looks up a transaction the caller owns. `owner_id` is the only key.
    pub async fn find_owned(pool: &DbPool, id: i64, owner_id: &str) -> Result<Option<Transaction>> {
        let tx = crate::db_fetch_optional!(
            pool,
            Transaction,
            &format!(
                "SELECT {TX_COLUMNS} FROM transactions t \
             LEFT JOIN finance_categories c ON c.id = t.category_id \
             WHERE t.id = ? AND t.owner_id = ?"
            ),
            id,
            owner_id
        )?;

        Ok(tx)
    }

    pub async fn list(
        pool: &DbPool,
        owner_id: &str,
        direction: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Transaction>> {
        let filter = if direction.is_some() {
            "AND t.direction = ?"
        } else {
            ""
        };
        let sql = format!(
            "SELECT {TX_COLUMNS} FROM transactions t \
             LEFT JOIN finance_categories c ON c.id = t.category_id \
             WHERE t.owner_id = ? {filter} \
             ORDER BY t.occurred_at DESC, t.id DESC LIMIT ? OFFSET ?"
        );

        // The filter is optional, and a macro call cannot bind conditionally, so
        // the branch is explicit. The SQL is unchanged either way.
        let items = match direction {
            Some(dir) => {
                crate::db_fetch_all!(pool, Transaction, &sql, owner_id, dir, limit, offset)?
            }
            None => crate::db_fetch_all!(pool, Transaction, &sql, owner_id, limit, offset)?,
        };
        Ok(items)
    }

    pub async fn count(pool: &DbPool, owner_id: &str, direction: Option<&str>) -> Result<i64> {
        let filter = if direction.is_some() {
            "AND direction = ?"
        } else {
            ""
        };
        let sql = format!("SELECT COUNT(*) FROM transactions WHERE owner_id = ? {filter}");

        Ok(match direction {
            Some(dir) => crate::db_scalar_one!(pool, i64, &sql, owner_id, dir)?,
            None => crate::db_scalar_one!(pool, i64, &sql, owner_id)?,
        })
    }

    // One parameter per column, which is what an INSERT is. Clippy's threshold is 7
    // and this is a row; collapsing them into a struct would spell the row type a
    // second time, minus the fields the database fills in, and every caller would
    // build that struct inline at the call site instead. No clarity is gained.
    #[allow(clippy::too_many_arguments)]
    pub async fn update(
        pool: &DbPool,
        id: i64,
        direction: &str,
        amount_minor: i64,
        currency: &str,
        currency_exponent: i64,
        category_id: Option<i64>,
        note: Option<&str>,
        occurred_at: DateTime<Utc>,
    ) -> Result<Option<Transaction>> {
        let result = crate::db_execute!(
            pool,
            r#"
            UPDATE transactions
            SET direction = ?, amount_minor = ?, currency = ?, currency_exponent = ?,
                category_id = ?, note = ?, occurred_at = ?
            WHERE id = ?
            "#,
            direction,
            amount_minor,
            currency,
            currency_exponent,
            category_id,
            note,
            occurred_at,
            id
        )?;

        if result == 0 {
            return Ok(None);
        }
        Self::find(pool, id).await
    }

    pub async fn delete(pool: &DbPool, id: i64) -> Result<bool> {
        let result = crate::db_execute!(pool, "DELETE FROM transactions WHERE id = ?", id)?;

        Ok(result > 0)
    }

    /// Totals for a period, grouped by currency and then category.
    ///
    /// Currencies are never added together: without a rate source, summing EUR
    /// and JPY produces a number that means nothing.
    pub async fn summary(
        pool: &DbPool,
        owner_id: &str,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
    ) -> Result<Vec<CurrencySummary>> {
        let rows = crate::db_fetch_all!(
            pool,
            (String, i64, Option<String>, String, i64),
            r#"
            SELECT t.currency, t.currency_exponent, c.name, t.direction, SUM(t.amount_minor)
            FROM transactions t
            LEFT JOIN finance_categories c ON c.id = t.category_id
            WHERE t.owner_id = ? AND t.occurred_at >= ? AND t.occurred_at < ?
            GROUP BY t.currency, t.currency_exponent, c.name, t.direction
            ORDER BY t.currency ASC
            "#,
            owner_id,
            from,
            to
        )?;

        let mut summaries: Vec<CurrencySummary> = Vec::new();

        for (currency, exponent, category, direction, total) in rows {
            let entry = match summaries.iter_mut().find(|s| s.currency == currency) {
                Some(existing) => existing,
                None => {
                    summaries.push(CurrencySummary {
                        currency: currency.clone(),
                        currency_exponent: exponent,
                        expense_minor: 0,
                        income_minor: 0,
                        net_minor: 0,
                        categories: Vec::new(),
                    });
                    summaries.last_mut().expect("just pushed")
                }
            };

            let is_expense = direction == "expense";
            if is_expense {
                entry.expense_minor += total;
            } else {
                entry.income_minor += total;
            }

            match entry.categories.iter_mut().find(|c| c.category == category) {
                Some(bucket) => {
                    if is_expense {
                        bucket.expense_minor += total;
                    } else {
                        bucket.income_minor += total;
                    }
                }
                None => entry.categories.push(CategoryTotal {
                    category,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;
    use chrono::Duration;

    async fn setup() -> DbPool {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();
        pool
    }

    // Mirrors `create_transaction`, so it carries the same column-per-argument
    // shape for the same reason.
    #[allow(clippy::too_many_arguments)]
    async fn log(
        pool: &DbPool,
        owner: &str,
        direction: &str,
        minor: i64,
        currency: &str,
        exponent: i64,
        category_id: Option<i64>,
        when: DateTime<Utc>,
    ) -> Transaction {
        FinanceRepository::create_transaction(
            pool,
            "web",
            owner,
            "tester",
            direction,
            minor,
            currency,
            exponent,
            category_id,
            Some("note"),
            None,
            None,
            when,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_category_names_are_unique_per_owner_not_globally() {
        let pool = setup().await;

        assert!(FinanceRepository::create_category(&pool, "@a:x", "food")
            .await
            .unwrap()
            .is_some());
        // Same name, same owner: rejected.
        assert!(FinanceRepository::create_category(&pool, "@a:x", "food")
            .await
            .unwrap()
            .is_none());
        // Same name, different owner: fine — categories are personal.
        assert!(FinanceRepository::create_category(&pool, "@b:x", "food")
            .await
            .unwrap()
            .is_some());
    }

    /// Deleting a category must never destroy financial records.
    #[tokio::test]
    async fn test_deleting_a_category_uncategorises_rather_than_deletes() {
        let pool = setup().await;
        let cat = FinanceRepository::create_category(&pool, "@a:x", "food")
            .await
            .unwrap()
            .unwrap();
        let tx = log(
            &pool,
            "@a:x",
            "expense",
            1550,
            "EUR",
            2,
            Some(cat.id),
            Utc::now(),
        )
        .await;

        let orphaned = FinanceRepository::delete_category(&pool, "@a:x", cat.id)
            .await
            .unwrap();
        assert_eq!(orphaned, Some(1));

        let still_there = FinanceRepository::find(&pool, tx.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(still_there.amount_minor, 1550, "the entry must survive");
        assert!(
            still_there.category_id.is_none(),
            "and be left uncategorised"
        );
    }

    #[tokio::test]
    async fn test_cannot_delete_another_owners_category() {
        let pool = setup().await;
        let cat = FinanceRepository::create_category(&pool, "@a:x", "food")
            .await
            .unwrap()
            .unwrap();

        assert!(FinanceRepository::delete_category(&pool, "@b:x", cat.id)
            .await
            .unwrap()
            .is_none());
        assert_eq!(
            FinanceRepository::list_categories(&pool, "@a:x")
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn test_ledger_is_isolated_per_owner() {
        let pool = setup().await;
        log(&pool, "@a:x", "expense", 100, "EUR", 2, None, Utc::now()).await;
        let theirs = log(&pool, "@b:x", "expense", 200, "EUR", 2, None, Utc::now()).await;

        let mine = FinanceRepository::list(&pool, "@a:x", None, 25, 0)
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].amount_minor, 100);

        assert!(FinanceRepository::find_owned(&pool, theirs.id, "@a:x")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_direction_filter() {
        let pool = setup().await;
        log(&pool, "@a:x", "expense", 100, "EUR", 2, None, Utc::now()).await;
        log(&pool, "@a:x", "income", 900, "EUR", 2, None, Utc::now()).await;

        let expenses = FinanceRepository::list(&pool, "@a:x", Some("expense"), 25, 0)
            .await
            .unwrap();
        assert_eq!(expenses.len(), 1);
        assert_eq!(expenses[0].direction, "expense");
        assert_eq!(
            FinanceRepository::count(&pool, "@a:x", Some("income"))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            FinanceRepository::count(&pool, "@a:x", None).await.unwrap(),
            2
        );
    }

    /// Currencies must never be added together — without a rate source the sum
    /// would be meaningless.
    #[tokio::test]
    async fn test_summary_keeps_currencies_apart() {
        let pool = setup().await;
        let now = Utc::now();
        log(&pool, "@a:x", "expense", 1550, "EUR", 2, None, now).await;
        log(&pool, "@a:x", "expense", 450, "EUR", 2, None, now).await;
        log(&pool, "@a:x", "income", 300000, "JPY", 0, None, now).await;

        let summaries = FinanceRepository::summary(
            &pool,
            "@a:x",
            now - Duration::days(1),
            now + Duration::days(1),
        )
        .await
        .unwrap();

        assert_eq!(summaries.len(), 2);

        let eur = summaries.iter().find(|s| s.currency == "EUR").unwrap();
        assert_eq!(eur.expense_minor, 2000);
        assert_eq!(eur.income_minor, 0);
        assert_eq!(eur.net_minor, -2000);

        let jpy = summaries.iter().find(|s| s.currency == "JPY").unwrap();
        assert_eq!(jpy.income_minor, 300000);
        assert_eq!(jpy.currency_exponent, 0);
        assert_eq!(jpy.net_minor, 300000);
    }

    #[tokio::test]
    async fn test_summary_groups_by_category_including_uncategorised() {
        let pool = setup().await;
        let now = Utc::now();
        let food = FinanceRepository::create_category(&pool, "@a:x", "food")
            .await
            .unwrap()
            .unwrap();

        log(&pool, "@a:x", "expense", 1000, "EUR", 2, Some(food.id), now).await;
        log(&pool, "@a:x", "expense", 250, "EUR", 2, Some(food.id), now).await;
        log(&pool, "@a:x", "expense", 700, "EUR", 2, None, now).await;

        let summaries = FinanceRepository::summary(
            &pool,
            "@a:x",
            now - Duration::days(1),
            now + Duration::days(1),
        )
        .await
        .unwrap();

        let eur = &summaries[0];
        assert_eq!(eur.expense_minor, 1950);

        let food_bucket = eur
            .categories
            .iter()
            .find(|c| c.category.as_deref() == Some("food"))
            .unwrap();
        assert_eq!(food_bucket.expense_minor, 1250);

        let uncategorised = eur
            .categories
            .iter()
            .find(|c| c.category.is_none())
            .unwrap();
        assert_eq!(uncategorised.expense_minor, 700);
    }

    /// The period is half-open: an entry exactly on the end boundary belongs to
    /// the next period, not this one.
    #[tokio::test]
    async fn test_summary_period_is_half_open() {
        let pool = setup().await;
        let start = Utc::now();
        let end = start + Duration::days(1);

        log(&pool, "@a:x", "expense", 100, "EUR", 2, None, start).await;
        log(&pool, "@a:x", "expense", 500, "EUR", 2, None, end).await;

        let summaries = FinanceRepository::summary(&pool, "@a:x", start, end)
            .await
            .unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].expense_minor, 100);
    }
}
