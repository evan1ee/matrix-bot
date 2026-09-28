use crate::accounts::Account;
use crate::db::DbPool;

pub struct AccountRepository;

impl AccountRepository {
    pub async fn create(
        pool: &DbPool,
        display_name: &str,
        status: &str,
    ) -> Result<String, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::core::create(
            pool,
            display_name,
            status,
        )
        .await
        .map(|id| id.into_inner())
    }

    pub async fn create_with_id(
        pool: &DbPool,
        account_id: &str,
        display_name: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::core::create_with_id(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
            display_name,
            status,
        )
        .await
    }

    pub async fn delete(pool: &DbPool, account_id: &str) -> Result<(), sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::core::delete(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    pub async fn find(pool: &DbPool, account_id: &str) -> Result<Option<Account>, sqlx::Error> {
        let domain_opt = crate::infrastructure::postgres::repositories::accounts::core::find(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await?;
        Ok(domain_opt.map(|a| Account {
            id: a.id.into_inner(),
            display_name: a.display_name,
            status: a.status,
            created_at: a.created_at,
        }))
    }

    pub async fn find_by_mxid(pool: &DbPool, mxid: &str) -> Result<Option<String>, sqlx::Error> {
        let opt =
            crate::infrastructure::postgres::repositories::accounts::core::find_by_mxid(pool, mxid)
                .await?;
        Ok(opt.map(|id| id.into_inner()))
    }

    pub async fn linked_mxids(pool: &DbPool, account_id: &str) -> Result<Vec<String>, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::core::linked_mxids(
            pool,
            &crate::domain::accounts::AccountId::new(account_id),
        )
        .await
    }

    pub async fn resolve_or_provision(pool: &DbPool, mxid: &str) -> Result<String, sqlx::Error> {
        crate::infrastructure::postgres::repositories::accounts::core::resolve_or_provision(
            pool, mxid,
        )
        .await
        .map(|id| id.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::STATUS_ACTIVE;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_ids_are_prefixed_and_unique() {
        let pool = db().await;
        let a = AccountRepository::create(&pool, "a", STATUS_ACTIVE)
            .await
            .unwrap();
        let b = AccountRepository::create(&pool, "b", STATUS_ACTIVE)
            .await
            .unwrap();

        assert!(a.starts_with("acct_"), "got {a}");
        assert_eq!(a.len(), "acct_".len() + 32);
        assert_ne!(a, b);
    }

    /// An unknown MXID must get exactly one account, and a second message from
    /// the same MXID must land on it rather than making another.
    #[tokio::test]
    async fn test_provisioning_is_stable_for_a_repeated_sender() {
        let pool = db().await;

        let first = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let second = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();

        assert_eq!(first, second);

        let count = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM accounts"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "a repeated sender must not accumulate accounts");
    }

    /// A provisioned account is unclaimed and named after the localpart.
    #[tokio::test]
    async fn test_provisioned_account_is_unclaimed() {
        let pool = db().await;
        let id = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();

        let account = AccountRepository::find(&pool, &id).await.unwrap().unwrap();
        assert!(account.is_unclaimed());
        assert_eq!(account.display_name, "evan");
    }

    /// Two different senders must never share an account.
    #[tokio::test]
    async fn test_two_senders_get_two_accounts() {
        let pool = db().await;
        let a = AccountRepository::resolve_or_provision(&pool, "@a:x")
            .await
            .unwrap();
        let b = AccountRepository::resolve_or_provision(&pool, "@b:x")
            .await
            .unwrap();
        assert_ne!(a, b);
    }

    /// The sync loop does not serialise event handlers, so two messages from a
    /// brand-new MXID can resolve concurrently. Both must agree on one account,
    /// and no orphan may be left behind — a stranded account would silently own
    /// whichever rows the losing task had already written.
    #[tokio::test]
    async fn test_concurrent_first_messages_converge_on_one_account() {
        let pool = db().await;

        let mut results = Vec::new();
        for _ in 0..8 {
            let pool = pool.clone();
            results.push(tokio::spawn(async move {
                AccountRepository::resolve_or_provision(&pool, "@racer:x")
                    .await
                    .unwrap()
            }));
        }

        let mut ids = Vec::new();
        for handle in results {
            ids.push(handle.await.unwrap());
        }

        assert!(
            ids.windows(2).all(|w| w[0] == w[1]),
            "resolved to different accounts: {ids:?}"
        );

        let accounts = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM accounts"#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(accounts, 1, "the losing task left an orphan account behind");
    }

    #[tokio::test]
    async fn test_linked_mxids_lists_every_route_to_an_account() {
        let pool = db().await;
        let account = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();

        sqlx::query!(
            "INSERT INTO matrix_links (mxid, account_id) VALUES ($1, $2)",
            "@evan:work.org",
            &account
        )
        .execute(&pool)
        .await
        .unwrap();

        let mxids = AccountRepository::linked_mxids(&pool, &account)
            .await
            .unwrap();
        assert_eq!(mxids.len(), 2);
        assert!(mxids.contains(&"@evan:work.org".to_string()));
    }

    /// One MXID may reach only one account. This is the schema-level guarantee
    /// the whole linking design rests on, so it is asserted directly.
    #[tokio::test]
    async fn test_an_mxid_cannot_be_linked_to_two_accounts() {
        let pool = db().await;
        let first = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();
        let other = AccountRepository::create(&pool, "other", STATUS_ACTIVE)
            .await
            .unwrap();
        assert_ne!(first, other);

        let duplicate = sqlx::query!(
            "INSERT INTO matrix_links (mxid, account_id) VALUES ($1, $2)",
            "@evan:mbm.one",
            &other
        )
        .execute(&pool)
        .await;

        assert!(
            duplicate.is_err(),
            "the primary key must reject a second link"
        );
    }
}
