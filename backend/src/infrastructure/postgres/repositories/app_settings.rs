//! Persistence for `app_settings` (runtime application configuration).

use chrono::{DateTime, Utc};
use sqlx::PgExecutor;

use crate::domain::accounts::AccountId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingRow {
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
    pub updated_by: Option<uuid::Uuid>,
}

/// Reads a single setting row by its unique key.
pub async fn get_setting(
    conn: impl PgExecutor<'_>,
    key: &str,
) -> Result<Option<SettingRow>, sqlx::Error> {
    sqlx::query_as!(
        SettingRow,
        "SELECT key, value, updated_at, updated_by FROM app_settings WHERE key = $1",
        key,
    )
    .fetch_optional(conn)
    .await
}

/// Lists all persisted runtime settings, ordered by key.
pub async fn list_settings(conn: impl PgExecutor<'_>) -> Result<Vec<SettingRow>, sqlx::Error> {
    sqlx::query_as!(
        SettingRow,
        "SELECT key, value, updated_at, updated_by FROM app_settings ORDER BY key",
    )
    .fetch_all(conn)
    .await
}

/// Inserts or updates a runtime setting value.
pub async fn upsert_setting(
    conn: impl PgExecutor<'_>,
    key: &str,
    value: &serde_json::Value,
    updated_by: Option<&AccountId>,
) -> Result<(), sqlx::Error> {
    let updated_by_uuid = updated_by.map(|a| a.uuid());
    sqlx::query!(
        "INSERT INTO app_settings (key, value, updated_at, updated_by) \
         VALUES ($1, $2, CURRENT_TIMESTAMP, $3) \
         ON CONFLICT (key) DO UPDATE SET \
            value = EXCLUDED.value, \
            updated_at = CURRENT_TIMESTAMP, \
            updated_by = EXCLUDED.updated_by",
        key,
        value,
        updated_by_uuid,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes a persisted setting row, reverting it to environment or default fallback.
pub async fn delete_setting(conn: impl PgExecutor<'_>, key: &str) -> Result<bool, sqlx::Error> {
    let res = sqlx::query!("DELETE FROM app_settings WHERE key = $1", key)
        .execute(conn)
        .await?;
    Ok(res.rows_affected() > 0)
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
    async fn test_app_settings_upsert_and_read_roundtrip() {
        let p = pool().await;
        let admin_id = AccountId::from_seed("admin");
        crate::db::pool::seed_account(&p, &admin_id).await;

        let val = serde_json::json!("gpt-4o");
        upsert_setting(&p, "llm.default_model", &val, Some(&admin_id))
            .await
            .unwrap();

        let row = get_setting(&p, "llm.default_model")
            .await
            .unwrap()
            .expect("row should exist");
        assert_eq!(row.key, "llm.default_model");
        assert_eq!(row.value, val);
        assert_eq!(row.updated_by, Some(admin_id.uuid()));

        // Update with new value
        let new_val = serde_json::json!("gpt-4o-mini");
        upsert_setting(&p, "llm.default_model", &new_val, Some(&admin_id))
            .await
            .unwrap();

        let updated = get_setting(&p, "llm.default_model")
            .await
            .unwrap()
            .expect("row should exist");
        assert_eq!(updated.value, new_val);

        // Delete
        let deleted = delete_setting(&p, "llm.default_model").await.unwrap();
        assert!(deleted);

        let none = get_setting(&p, "llm.default_model").await.unwrap();
        assert!(none.is_none());
    }
}
