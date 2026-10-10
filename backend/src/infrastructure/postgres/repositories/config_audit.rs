//! Persistence for `config_audit_log` (administrative configuration audit ledger).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigAuditRow {
    pub id: i64,
    pub actor_id: Option<uuid::Uuid>,
    pub action: String,
    pub target_type: String,
    pub target_key: String,
    pub outcome: String,
    pub detail: Option<serde_json::Value>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Appends a new configuration audit entry.
pub async fn insert_audit_entry(
    conn: impl PgExecutor<'_>,
    actor_id: Option<&AccountId>,
    action: &str,
    target_type: &str,
    target_key: &str,
    outcome: &str,
    detail: Option<&serde_json::Value>,
) -> Result<i64, sqlx::Error> {
    let actor_id_uuid = actor_id.map(|a| a.uuid());
    let id: i64 = sqlx::query_scalar!(
        "INSERT INTO config_audit_log (actor_id, action, target_type, target_key, outcome, detail) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        actor_id_uuid,
        action,
        target_type,
        target_key,
        outcome,
        detail,
    )
    .fetch_one(conn)
    .await?;
    Ok(id)
}

/// Fetches a paginated slice of audit entries ordered newest first.
pub async fn list_audit_entries(
    conn: impl PgExecutor<'_>,
    limit: i64,
    offset: i64,
) -> Result<Vec<ConfigAuditRow>, sqlx::Error> {
    sqlx::query_as!(
        ConfigAuditRow,
        "SELECT id, actor_id, action, target_type, target_key, outcome, detail, created_at \
         FROM config_audit_log ORDER BY id DESC LIMIT $1 OFFSET $2",
        limit,
        offset,
    )
    .fetch_all(conn)
    .await
}

/// Returns the total count of administrative audit log entries.
pub async fn count_audit_entries(conn: impl PgExecutor<'_>) -> Result<i64, sqlx::Error> {
    let count: i64 = sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM config_audit_log"#)
        .fetch_one(conn)
        .await?;
    Ok(count)
}

/// Remaps administrative audit and configuration records from an absorbed account into the survivor.
pub async fn remap_admin_audit_and_settings(
    conn: &mut sqlx::PgConnection,
    from: &AccountId,
    into: &AccountId,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "UPDATE config_audit_log SET actor_id = $1 WHERE actor_id = $2",
        into.as_uuid(),
        from.as_uuid(),
    )
    .execute(&mut *conn)
    .await?;

    sqlx::query!(
        "UPDATE app_settings SET updated_by = $1 WHERE updated_by = $2",
        into.as_uuid(),
        from.as_uuid(),
    )
    .execute(&mut *conn)
    .await?;

    sqlx::query!(
        "UPDATE provider_secrets SET updated_by = $1 WHERE updated_by = $2",
        into.as_uuid(),
        from.as_uuid(),
    )
    .execute(&mut *conn)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn pool() -> PgPool {
        let p = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&p).await;
        p
    }

    #[tokio::test]
    async fn test_config_audit_insert_and_paginate() {
        let p = pool().await;
        let admin = AccountId::from_seed("admin");
        crate::db::pool::seed_account(&p, &admin).await;

        let detail = serde_json::json!({ "model": "gpt-4o" });
        let id1 = insert_audit_entry(
            &p,
            Some(&admin),
            "setting_updated",
            "setting",
            "llm.default_model",
            "success",
            Some(&detail),
        )
        .await
        .unwrap();

        let id2 = insert_audit_entry(
            &p,
            None,
            "config_reloaded",
            "system",
            "system",
            "success",
            None,
        )
        .await
        .unwrap();

        assert!(id2 > id1);

        let count = count_audit_entries(&p).await.unwrap();
        assert_eq!(count, 2);

        let entries = list_audit_entries(&p, 10, 0).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].action, "config_reloaded");
        assert_eq!(entries[1].action, "setting_updated");
        assert_eq!(entries[1].actor_id, Some(admin.uuid()));
    }
}
