//! Security events audit log repository.

use chrono::{DateTime, Utc};

use crate::domain::accounts::{AccountId, SecurityEvent, SecurityEventKind};

struct SecurityEventRow {
    id: i64,
    account_id: String,
    kind: String,
    detail: Option<String>,
    created_at: Option<DateTime<Utc>>,
}

impl From<SecurityEventRow> for SecurityEvent {
    fn from(row: SecurityEventRow) -> Self {
        SecurityEvent {
            id: row.id,
            account_id: AccountId::new(row.account_id),
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
        account_id.as_str(),
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

pub async fn list(
    conn: impl sqlx::PgExecutor<'_>,
    account_id: &AccountId,
    limit: i64,
) -> Result<Vec<SecurityEvent>, sqlx::Error> {
    let rows = sqlx::query_as!(
        SecurityEventRow,
        "SELECT id, account_id, kind, detail, created_at FROM security_events \
         WHERE account_id = $1 ORDER BY created_at DESC, id DESC LIMIT $2",
        account_id.as_str(),
        limit,
    )
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(Into::into).collect())
}
