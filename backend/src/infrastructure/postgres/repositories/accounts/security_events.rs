//! Security events audit log repository.

use chrono::{DateTime, Utc};

use crate::domain::accounts::{AccountId, SecurityEvent, SecurityEventKind};
use crate::domain::channels::{Channel, PlatformIdentity};

struct SecurityEventRow {
    id: i64,
    account_id: uuid::Uuid,
    kind: String,
    detail: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

impl From<SecurityEventRow> for SecurityEvent {
    fn from(row: SecurityEventRow) -> Self {
        SecurityEvent {
            id: row.id,
            account_id: AccountId::from_uuid(row.account_id),
            kind: row.kind,
            detail: row.detail,
            created_at: row.created_at,
        }
    }
}

pub async fn record(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    kind: SecurityEventKind,
    detail: Option<&str>,
) {
    let result = sqlx::query!(
        "INSERT INTO security_events (account_id, kind, detail) VALUES ($1, $2, $3)",
        account_id.as_uuid(),
        kind.as_str(),
        detail,
    )
    .execute(conn)
    .await;

    if let Err(err) = result {
        tracing::error!(
            account_id = %account_id,
            kind = kind.as_str(),
            error = %err,
            "Could not record security event"
        );
    }
}

/// The account an identity was last unlinked from, if it ever was.
///
/// Two audit shapes are honoured: `identity_unlinked` (detail
/// `<platform>:<user_id>`) and, for Matrix only, the historic `matrix_unlinked`
/// rows whose detail is the bare MXID (A3).
pub async fn last_unlinked_from(
    conn: impl sqlx::PgExecutor<'_>,
    identity: &PlatformIdentity,
) -> Result<Option<AccountId>, sqlx::Error> {
    let user_id = identity.user_id().as_str();
    let account = sqlx::query_scalar!(
        r#"SELECT account_id AS "account_id: uuid::Uuid" FROM security_events
           WHERE (kind = 'matrix_unlinked' AND $3 AND detail = $1)
              OR (kind = 'identity_unlinked' AND detail = $2)
           ORDER BY id DESC LIMIT 1"#,
        user_id,
        format!("{}:{user_id}", identity.channel()),
        identity.channel() == Channel::Matrix,
    )
    .fetch_optional(conn)
    .await?;
    Ok(account.map(AccountId::from_uuid))
}

pub async fn list(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    limit: i64,
) -> Result<Vec<SecurityEvent>, sqlx::Error> {
    let rows = sqlx::query_as!(
        SecurityEventRow,
        "SELECT id, account_id, kind, detail, created_at FROM security_events \
         WHERE account_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
        account_id.as_uuid(),
        limit,
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
    use crate::infrastructure::postgres::repositories::accounts::core;
    use sqlx::PgPool;

    async fn db() -> PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    #[tokio::test]
    async fn test_events_are_recorded_and_listed_newest_first() {
        let pool = db().await;
        let acct = core::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();

        record(&pool, &acct, SecurityEventKind::SignedIn, None).await;
        record(
            &pool,
            &acct,
            SecurityEventKind::MatrixLinked,
            Some("@evan:mbm.one"),
        )
        .await;

        let events = list(&pool, &acct, 10).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "matrix_linked");
        assert_eq!(events[0].detail.as_deref(), Some("@evan:mbm.one"));
    }

    /// One account's log must never show another's.
    #[tokio::test]
    async fn test_the_log_is_per_account() {
        let pool = db().await;
        let mine = core::create(&pool, "mine", STATUS_ACTIVE).await.unwrap();
        let theirs = core::create(&pool, "theirs", STATUS_ACTIVE).await.unwrap();

        record(&pool, &theirs, SecurityEventKind::SignedIn, None).await;

        assert!(list(&pool, &mine, 10).await.unwrap().is_empty());
        assert_eq!(list(&pool, &theirs, 10).await.unwrap().len(), 1);
    }

    /// Recording must not be able to fail an action that already happened.
    #[tokio::test]
    async fn test_recording_against_a_broken_table_does_not_panic() {
        let pool = crate::db::pool::test_pool().await;
        // No migrations: the table does not exist.
        record(
            &pool,
            &AccountId::from_seed("acct_a"),
            SecurityEventKind::SignedIn,
            None,
        )
        .await;
    }
}
