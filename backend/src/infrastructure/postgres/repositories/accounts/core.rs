//! Accounts repository: persistence for the `accounts` table.

use crate::domain::accounts::{Account, AccountId};
use chrono::{DateTime, Utc};

struct AccountRow {
    id: uuid::Uuid,
    display_name: String,
    status: String,
    role: String,
    created_at: Option<DateTime<Utc>>,
}

impl From<AccountRow> for Account {
    fn from(row: AccountRow) -> Self {
        Account {
            id: AccountId::new(row.id),
            display_name: row.display_name,
            status: row.status,
            role: row.role.parse().unwrap_or_default(),
            created_at: row.created_at,
        }
    }
}

/// Creates an account, minting the id from the OS CSPRNG.
pub async fn create(
    conn: impl sqlx::PgExecutor<'_>,
    display_name: &str,
    status: &str,
) -> Result<AccountId, sqlx::Error> {
    let id = uuid::Uuid::new_v4();
    let row: uuid::Uuid = sqlx::query_scalar!(
        "INSERT INTO accounts (id, display_name, status) VALUES ($1, $2, $3) RETURNING id",
        id,
        display_name,
        status,
    )
    .fetch_one(conn)
    .await?;
    Ok(AccountId::new(row))
}

/// Creates an account under an id the caller already holds (e.g. for WebAuthn user handle pre-creation).
pub async fn create_with_id(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    display_name: &str,
    status: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO accounts (id, display_name, status) VALUES ($1, $2, $3)",
        account_id.as_uuid(),
        display_name,
        status,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Activates an existing account or creates a new active account with this id.
pub async fn activate_or_create(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    display_name: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO accounts (id, display_name, status) VALUES ($1, $2, 'active') \
         ON CONFLICT (id) DO UPDATE SET status = 'active'",
        account_id.as_uuid(),
        display_name,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Finds an account by ID.
pub async fn find(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Option<Account>, sqlx::Error> {
    let row = sqlx::query_as!(
        AccountRow,
        "SELECT id, display_name, status, role, created_at FROM accounts WHERE id = $1",
        account_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Into::into))
}

/// Updates an account's role.
pub async fn update_role(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    role: crate::domain::accounts::AccountRole,
) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!(
        "UPDATE accounts SET role = $1 WHERE id = $2",
        role.as_str(),
        account_id.as_uuid(),
    )
    .execute(conn)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Takes the account row lock for the rest of the transaction, serialising
/// read-modify-write decisions about the account (e.g. which sign-in routes
/// may still be removed). `false` when the account does not exist.
pub async fn lock(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query_scalar!(
        "SELECT id FROM accounts WHERE id = $1 FOR UPDATE",
        account_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.is_some())
}

/// Total count of accounts.
pub async fn count(conn: impl sqlx::PgExecutor<'_>) -> Result<i64, sqlx::Error> {
    let count = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM accounts"#)
        .fetch_one(conn)
        .await?;
    Ok(count)
}

/// Lists IDs of all active accounts.
pub async fn list_active_ids(
    conn: impl sqlx::PgExecutor<'_>,
) -> Result<Vec<AccountId>, sqlx::Error> {
    let rows: Vec<uuid::Uuid> = sqlx::query_scalar!(
        r#"SELECT id AS "id: uuid::Uuid" FROM accounts WHERE status = 'active'"#
    )
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().map(AccountId::new).collect())
}

/// Deletes an account by ID.
pub async fn delete(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<(), sqlx::Error> {
    sqlx::query!("DELETE FROM accounts WHERE id = $1", account_id.as_uuid())
        .execute(conn)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::domain::channels::{Channel, PlatformIdentity};
    use crate::infrastructure::postgres::repositories::identities;
    use sqlx::PgPool;

    /// Test shorthand: provisions the account for a Matrix sender.
    async fn resolve_or_provision(pool: &PgPool, mxid: &str) -> Result<AccountId, sqlx::Error> {
        identities::resolve_or_provision(pool, &PlatformIdentity::matrix(mxid).unwrap()).await
    }

    async fn pool() -> PgPool {
        let p = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&p).await;
        p
    }

    #[tokio::test]
    async fn test_create_and_find_account() {
        let p = pool().await;
        let acct_id = create(&p, "alice", STATUS_ACTIVE).await.unwrap();
        let acct = find(&p, &acct_id).await.unwrap().expect("account exists");
        assert_eq!(acct.id, acct_id);
        assert_eq!(acct.display_name, "alice");
        assert_eq!(acct.status, STATUS_ACTIVE);
    }

    #[tokio::test]
    async fn test_resolve_or_provision_is_idempotent() {
        let p = pool().await;
        let first = resolve_or_provision(&p, "@bob:matrix.org").await.unwrap();
        let second = resolve_or_provision(&p, "@bob:matrix.org").await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn test_ids_are_prefixed_and_unique() {
        let pool = pool().await;
        let a = create(&pool, "a", STATUS_ACTIVE).await.unwrap();
        let b = create(&pool, "b", STATUS_ACTIVE).await.unwrap();

        let (a_text, b_text) = (a.to_string(), b.to_string());
        assert!(a_text.starts_with("acct_"), "got {a_text}");
        assert_eq!(a_text.len(), "acct_".len() + 32);
        assert_ne!(a_text, b_text);
        assert_ne!(a, b);
    }

    /// An unknown MXID must get exactly one account, and a second message from
    /// the same MXID must land on it rather than making another.
    #[tokio::test]
    async fn test_provisioning_is_stable_for_a_repeated_sender() {
        let pool = pool().await;

        let first = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let second = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();

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
        let pool = pool().await;
        let id = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();

        let account = find(&pool, &id).await.unwrap().unwrap();
        assert!(account.is_unclaimed());
        assert_eq!(account.display_name, "evan");
    }

    /// Two different senders must never share an account.
    #[tokio::test]
    async fn test_two_senders_get_two_accounts() {
        let pool = pool().await;
        let a = resolve_or_provision(&pool, "@a:x").await.unwrap();
        let b = resolve_or_provision(&pool, "@b:x").await.unwrap();
        assert_ne!(a, b);
    }

    #[tokio::test]
    async fn test_linked_mxids_lists_every_route_to_an_account() {
        let pool = pool().await;
        let account = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let work = PlatformIdentity::matrix("@evan:work.org").unwrap();
        assert!(identities::bind(&pool, &account, &work).await.unwrap());

        let mxids = identities::user_ids(&pool, &account, Channel::Matrix)
            .await
            .unwrap();
        assert_eq!(mxids.len(), 2);
        assert!(mxids.contains(&"@evan:work.org".to_string()));
    }

    /// One MXID may reach only one account. This is the schema-level guarantee
    /// the whole linking design rests on, so it is asserted directly.
    #[tokio::test]
    async fn test_an_mxid_cannot_be_linked_to_two_accounts() {
        let pool = pool().await;
        let first = resolve_or_provision(&pool, "@evan:mbm.one").await.unwrap();
        let other = create(&pool, "other", STATUS_ACTIVE).await.unwrap();
        assert_ne!(first, other);

        let duplicate = sqlx::query!(
            "INSERT INTO platform_identities (account_id, platform, platform_user_id) \
             VALUES ($1, 'matrix', $2)",
            other.as_uuid(),
            "@evan:mbm.one",
        )
        .execute(&pool)
        .await;

        assert!(
            duplicate.is_err(),
            "the unique key must reject a second link"
        );
    }
}
