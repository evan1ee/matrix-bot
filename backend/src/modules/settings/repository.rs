use sqlx::SqlitePool;

use crate::modules::settings::models::UserSettings;

pub struct SettingsRepository;

impl SettingsRepository {
    /// The caller's settings, or the defaults if they have never saved any.
    ///
    /// A missing row is the normal case, not an error: settings are written only
    /// when a user changes one, so every user starts without a row and must
    /// still get a usable answer.
    pub async fn get(pool: &SqlitePool, owner_id: &str) -> Result<UserSettings, sqlx::Error> {
        let found = sqlx::query_as::<_, UserSettings>(
            "SELECT timezone, currency, dev_room_id, max_history_turns FROM user_settings WHERE owner_id = ?",
        )
        .bind(owner_id)
        .fetch_optional(pool)
        .await?;

        Ok(found.unwrap_or_default())
    }

    /// Reads settings for a command handler, degrading to the defaults on a
    /// database error.
    ///
    /// The Matrix command path has no way to surface a settings lookup failure
    /// that would not also destroy the reply the user actually asked for, so a
    /// failed read renders in UTC/USD and logs, rather than aborting.
    pub async fn get_or_default(pool: &SqlitePool, owner_id: &str) -> UserSettings {
        match Self::get(pool, owner_id).await {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!("Falling back to default settings for {}: {:?}", owner_id, err);
                UserSettings::default()
            }
        }
    }

    /// Writes preferences, creating the row if it does not exist.
    pub async fn upsert(
        pool: &SqlitePool,
        owner_id: &str,
        settings: &UserSettings,
    ) -> Result<UserSettings, sqlx::Error> {
        sqlx::query(
            "INSERT INTO user_settings (owner_id, timezone, currency, dev_room_id, max_history_turns, updated_at) \
             VALUES (?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT(owner_id) DO UPDATE SET \
                timezone = excluded.timezone, \
                currency = excluded.currency, \
                dev_room_id = excluded.dev_room_id, \
                max_history_turns = excluded.max_history_turns, \
                updated_at = CURRENT_TIMESTAMP",
        )
        .bind(owner_id)
        .bind(&settings.timezone)
        .bind(&settings.currency)
        .bind(&settings.dev_room_id)
        .bind(settings.max_history_turns)
        .execute(pool)
        .await?;

        Self::get(pool, owner_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        pool
    }

    /// Every user starts without a row, so this is the common path, not an edge.
    #[tokio::test]
    async fn test_a_user_with_no_row_gets_the_defaults() {
        let pool = pool().await;
        let settings = SettingsRepository::get(&pool, "@evan:mbm.one").await.unwrap();
        assert_eq!(settings, UserSettings::default());
    }

    #[tokio::test]
    async fn test_upsert_creates_then_updates_in_place() {
        let pool = pool().await;

        let created = SettingsRepository::upsert(
            &pool,
            "@evan:mbm.one",
            &UserSettings {
                timezone: "Asia/Shanghai".to_string(),
                currency: "CNY".to_string(),
                dev_room_id: Some("!dev:mbm.one".to_string()),
                max_history_turns: 15,
            },
        )
        .await
        .unwrap();
        assert_eq!(created.timezone, "Asia/Shanghai");
        assert_eq!(created.currency, "CNY");
        assert_eq!(created.dev_room_id.as_deref(), Some("!dev:mbm.one"));
        assert_eq!(created.max_history_turns, 15);

        let updated = SettingsRepository::upsert(
            &pool,
            "@evan:mbm.one",
            &UserSettings {
                timezone: "Europe/Berlin".to_string(),
                currency: "EUR".to_string(),
                dev_room_id: None,
                max_history_turns: 20,
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.timezone, "Europe/Berlin");
        assert_eq!(updated.dev_room_id, None);
        assert_eq!(updated.max_history_turns, 20);

        // The upsert must update the existing row, never accumulate rows.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    /// `owner_id` is the ownership key everywhere else in the system; settings
    /// must not be an exception, or one user's preference would render another's
    /// timestamps.
    #[tokio::test]
    async fn test_settings_are_isolated_per_owner() {
        let pool = pool().await;

        SettingsRepository::upsert(
            &pool,
            "@evan:mbm.one",
            &UserSettings {
                timezone: "Asia/Shanghai".to_string(),
                currency: "CNY".to_string(),
                dev_room_id: Some("!dev:mbm.one".to_string()),
                max_history_turns: 10,
            },
        )
        .await
        .unwrap();

        // The two identity spaces share a display name but never an owner_id.
        let other = SettingsRepository::get(&pool, "@web-evan:api").await.unwrap();
        assert_eq!(other, UserSettings::default());
    }
}
