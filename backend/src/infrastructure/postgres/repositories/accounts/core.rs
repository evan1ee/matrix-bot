//! Accounts repository: persistence for the `accounts` table and MXID provisioning.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::accounts::{display_name_for_mxid, Account, AccountId, STATUS_UNCLAIMED};

struct AccountRow {
    id: uuid::Uuid,
    display_name: String,
    status: String,
    created_at: Option<DateTime<Utc>>,
}

impl From<AccountRow> for Account {
    fn from(row: AccountRow) -> Self {
        Account {
            id: AccountId::new(row.id),
            display_name: row.display_name,
            status: row.status,
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
        "SELECT id, display_name, status, created_at FROM accounts WHERE id = $1",
        account_id.as_uuid(),
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(Into::into))
}

/// Looks up an account ID linked to an MXID.
pub async fn find_by_mxid(
    conn: impl sqlx::PgExecutor<'_>,
    mxid: &str,
) -> Result<Option<AccountId>, sqlx::Error> {
    let row = sqlx::query_scalar!(
        r#"SELECT account_id AS "account_id: uuid::Uuid" FROM matrix_links WHERE mxid = $1"#,
        mxid,
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(AccountId::new))
}

/// Every MXID linked to `account_id`, ordered by linked_at.
pub async fn linked_mxids(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar!(
        "SELECT mxid FROM matrix_links WHERE account_id = $1 ORDER BY linked_at",
        account_id.as_uuid(),
    )
    .fetch_all(conn)
    .await
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

/// Resolves or provisions an unclaimed account for an MXID.
pub async fn resolve_or_provision(pool: &PgPool, mxid: &str) -> Result<AccountId, sqlx::Error> {
    if let Some(existing) = find_by_mxid(pool, mxid).await? {
        return Ok(existing);
    }

    let account_id = create(pool, &display_name_for_mxid(mxid), STATUS_UNCLAIMED).await?;

    let inserted = sqlx::query!(
        "INSERT INTO matrix_links (mxid, account_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        mxid,
        account_id.as_uuid(),
    )
    .execute(pool)
    .await?
    .rows_affected()
        > 0;

    if inserted {
        Ok(account_id)
    } else {
        delete(pool, &account_id).await?;
        find_by_mxid(pool, mxid)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;

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
}
