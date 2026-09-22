use crate::accounts::{display_name_for_mxid, Account, STATUS_UNCLAIMED};
use crate::db::DbPool;

pub struct AccountRepository;

impl AccountRepository {
    /// Creates an account, letting SQLite mint the id.
    ///
    /// `randomblob(16)` rather than a Rust-side UUID because no random-number
    /// crate is a direct dependency of this project and none needs to be: the
    /// id has to be unguessable, not sortable or parseable, and SQLite already
    /// ships a CSPRNG. Prefixing with `acct_` keeps it obvious at a glance
    /// which identity space a stray string in a log belongs to.
    pub async fn create(
        pool: &DbPool,
        display_name: &str,
        status: &str,
    ) -> Result<String, sqlx::Error> {
        crate::db_scalar_one!(
            pool,
            String,
            "INSERT INTO accounts (id, display_name, status) \
             VALUES (?, ?, ?) \
             RETURNING id",
            crate::utils::random::new_account_id(),
            display_name,
            status
        )
    }

    /// Creates an account under an id the caller already holds.
    ///
    /// Passkey registration needs this: the account id *is* the WebAuthn user
    /// handle, so it has to exist before the challenge is issued — but the row
    /// must not, or an abandoned registration would leave an account nobody can
    /// ever sign in to.
    pub async fn create_with_id(
        pool: &DbPool,
        account_id: &str,
        display_name: &str,
        status: &str,
    ) -> Result<(), sqlx::Error> {
        crate::db_execute!(
            pool,
            "INSERT INTO accounts (id, display_name, status) VALUES (?, ?, ?)",
            account_id,
            display_name,
            status
        )?;
        Ok(())
    }

    pub async fn find(pool: &DbPool, account_id: &str) -> Result<Option<Account>, sqlx::Error> {
        crate::db_fetch_optional!(
            pool,
            Account,
            "SELECT id, display_name, status, created_at FROM accounts WHERE id = ?",
            account_id
        )
    }

    /// The account an MXID is linked to, or `None` if it has never been seen.
    pub async fn find_by_mxid(pool: &DbPool, mxid: &str) -> Result<Option<String>, sqlx::Error> {
        crate::db_scalar_optional!(
            pool,
            String,
            "SELECT account_id FROM matrix_links WHERE mxid = ?",
            mxid
        )
    }

    /// Every MXID that can reach `account_id`.
    ///
    /// Phase 3 fans security notifications out over exactly this list.
    pub async fn linked_mxids(pool: &DbPool, account_id: &str) -> Result<Vec<String>, sqlx::Error> {
        crate::db_scalar_all!(
            pool,
            String,
            "SELECT mxid FROM matrix_links WHERE account_id = ? ORDER BY linked_at",
            account_id
        )
    }

    /// The owner id for a Matrix sender, provisioning an unclaimed account the
    /// first time that MXID is seen.
    ///
    /// This is the whole of decision D1: the bot stays usable with no web
    /// signup, because a first-time sender gets a real account immediately
    /// rather than an error telling them to go register.
    ///
    /// The insert is `OR IGNORE` and the loser cleans up after itself, because
    /// two messages from the same new MXID can land in different tasks at the
    /// same time — the sync loop does not serialise event handlers. Without
    /// that, the two would provision two accounts, the second would overwrite
    /// the link, and everything written against the first would be orphaned.
    pub async fn resolve_or_provision(pool: &DbPool, mxid: &str) -> Result<String, sqlx::Error> {
        if let Some(existing) = Self::find_by_mxid(pool, mxid).await? {
            return Ok(existing);
        }

        let candidate = Self::create(pool, &display_name_for_mxid(mxid), STATUS_UNCLAIMED).await?;

        let claimed = crate::db_execute!(
            pool,
            "INSERT INTO matrix_links (mxid, account_id) VALUES (?, ?) ON CONFLICT DO NOTHING",
            mxid,
            &candidate
        )?;

        if claimed == 1 {
            return Ok(candidate);
        }

        // A concurrent call linked this MXID first. Drop the account we minted
        // — nothing references it yet — and use the winner's.
        crate::db_execute!(pool, "DELETE FROM accounts WHERE id = ?", &candidate)?;

        Self::find_by_mxid(pool, mxid)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::STATUS_ACTIVE;
    use crate::db::schema::run_migrations;

    async fn db() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
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

        let count: i64 = crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM accounts").unwrap();
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

        let accounts: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM accounts").unwrap();
        assert_eq!(accounts, 1, "the losing task left an orphan account behind");
    }

    #[tokio::test]
    async fn test_linked_mxids_lists_every_route_to_an_account() {
        let pool = db().await;
        let account = AccountRepository::resolve_or_provision(&pool, "@evan:mbm.one")
            .await
            .unwrap();

        crate::db_execute!(
            &pool,
            "INSERT INTO matrix_links (mxid, account_id) VALUES (?, ?)",
            "@evan:work.org",
            &account
        )
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

        let duplicate = crate::db_execute!(
            &pool,
            "INSERT INTO matrix_links (mxid, account_id) VALUES (?, ?)",
            "@evan:mbm.one",
            &other
        );

        assert!(
            duplicate.is_err(),
            "the primary key must reject a second link"
        );
    }
}
