//! PostgreSQL repository for finance entities: categories, transactions, analytics, and shares.

pub mod analytics;
pub mod categories;
pub mod transactions;

pub use analytics::*;
pub use categories::*;
pub use transactions::*;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use sqlx::PgPool;

    use crate::domain::accounts::AccountId;
    use crate::domain::finance::{Direction, NewTransaction};

    async fn setup() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    async fn insert_account(pool: &PgPool, owner: &AccountId) {
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'test', 'active') ON CONFLICT DO NOTHING",
            owner.as_uuid()
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    async fn log(
        pool: &PgPool,
        owner: &AccountId,
        direction: Direction,
        minor: i64,
        currency: &str,
        exponent: i64,
        category_id: Option<i64>,
        when: chrono::DateTime<Utc>,
    ) -> crate::domain::finance::Transaction {
        insert_account(pool, owner).await;
        let input = NewTransaction {
            direction,
            amount_minor: minor,
            currency,
            currency_exponent: exponent,
            category: None,
            note: Some("note"),
            room_id: "web",
            sender_name: "tester",
            occurred_at: when,
            source: None,
            create_missing_category: false,
        };
        transactions::create_transaction(
            &mut pool.acquire().await.unwrap(),
            owner,
            &input,
            category_id,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_category_names_are_unique_per_owner_not_globally() {
        let pool = setup().await;
        let owner_a = AccountId::from_seed("acct_a");
        let owner_b = AccountId::from_seed("acct_b");
        insert_account(&pool, &owner_a).await;
        insert_account(&pool, &owner_b).await;

        assert!(categories::create_category(&pool, &owner_a, "food")
            .await
            .unwrap()
            .is_some());
        // Same name, same owner: rejected by ON CONFLICT DO NOTHING
        assert!(categories::create_category(&pool, &owner_a, "food")
            .await
            .unwrap()
            .is_none());
        // Same name, different owner: fine — categories are personal
        assert!(categories::create_category(&pool, &owner_b, "food")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn test_deleting_a_category_uncategorises_rather_than_deletes() {
        let pool = setup().await;
        let owner = AccountId::from_seed("acct_a");
        insert_account(&pool, &owner).await;
        let cat = categories::create_category(&pool, &owner, "food")
            .await
            .unwrap()
            .unwrap();

        let tx = log(
            &pool,
            &owner,
            Direction::Expense,
            1550,
            "EUR",
            2,
            Some(cat.id),
            Utc::now(),
        )
        .await;

        let orphaned =
            categories::delete_category(&mut pool.acquire().await.unwrap(), &owner, cat.id)
                .await
                .unwrap();
        assert_eq!(orphaned, Some(1));

        let still_there = transactions::find(&pool, tx.id).await.unwrap().unwrap();
        assert_eq!(still_there.amount_minor, 1550, "the entry must survive");
        assert!(
            still_there.category_id.is_none(),
            "and be left uncategorised"
        );
    }

    #[tokio::test]
    async fn test_cannot_delete_another_owners_category() {
        let pool = setup().await;
        let owner_a = AccountId::from_seed("acct_a");
        let owner_b = AccountId::from_seed("acct_b");
        insert_account(&pool, &owner_a).await;
        insert_account(&pool, &owner_b).await;

        let cat = categories::create_category(&pool, &owner_a, "food")
            .await
            .unwrap()
            .unwrap();

        assert!(
            categories::delete_category(&mut pool.acquire().await.unwrap(), &owner_b, cat.id)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            categories::list_categories(&pool, &owner_a)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn test_ledger_is_isolated_per_owner() {
        let pool = setup().await;
        let owner_a = AccountId::from_seed("acct_a");
        let owner_b = AccountId::from_seed("acct_b");

        log(
            &pool,
            &owner_a,
            Direction::Expense,
            100,
            "EUR",
            2,
            None,
            Utc::now(),
        )
        .await;
        let theirs = log(
            &pool,
            &owner_b,
            Direction::Expense,
            200,
            "EUR",
            2,
            None,
            Utc::now(),
        )
        .await;

        let filter = crate::domain::finance::ListTransactionsFilter::default();
        let mine = transactions::list(&pool, &owner_a, &filter).await.unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].amount_minor, 100);

        assert!(transactions::find_owned(&pool, &owner_a, theirs.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn test_direction_filter() {
        let pool = setup().await;
        let owner = AccountId::from_seed("acct_a");

        log(
            &pool,
            &owner,
            Direction::Expense,
            100,
            "EUR",
            2,
            None,
            Utc::now(),
        )
        .await;
        log(
            &pool,
            &owner,
            Direction::Income,
            900,
            "EUR",
            2,
            None,
            Utc::now(),
        )
        .await;

        let filter = crate::domain::finance::ListTransactionsFilter {
            direction: Some(Direction::Expense),
            ..Default::default()
        };
        let expenses = transactions::list(&pool, &owner, &filter).await.unwrap();
        assert_eq!(expenses.len(), 1);
        assert_eq!(expenses[0].direction, Direction::Expense);

        assert_eq!(
            transactions::count(&pool, &owner, Some(Direction::Income))
                .await
                .unwrap(),
            1
        );
        assert_eq!(transactions::count(&pool, &owner, None).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn test_summary_keeps_currencies_apart() {
        let pool = setup().await;
        let owner = AccountId::from_seed("acct_a");
        let now = Utc::now();

        log(&pool, &owner, Direction::Expense, 1550, "EUR", 2, None, now).await;
        log(&pool, &owner, Direction::Expense, 450, "EUR", 2, None, now).await;
        log(
            &pool,
            &owner,
            Direction::Income,
            300000,
            "JPY",
            0,
            None,
            now,
        )
        .await;

        let summaries = transactions::summary(
            &pool,
            &owner,
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
}
