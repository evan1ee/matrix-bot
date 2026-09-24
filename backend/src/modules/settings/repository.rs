use crate::db::DbPool;
use crate::modules::settings::models::UserSettings;

pub struct SettingsRepository;

impl SettingsRepository {
    /// The caller's settings, or the defaults if they have never saved any.
    ///
    /// A missing row is the normal case, not an error: settings are written only
    /// when a user changes one, so every user starts without a row and must
    /// still get a usable answer.
    pub async fn get(pool: &DbPool, owner_id: &str) -> Result<UserSettings, sqlx::Error> {
        let found = crate::db_fetch_optional!(pool, UserSettings, "SELECT timezone, currency, dev_room_id, max_history_turns, briefing_enabled, briefing_time, tts_voice FROM user_settings WHERE owner_id = ?", owner_id)?;

        Ok(found.unwrap_or_default())
    }

    /// Reads settings for a command handler, degrading to the defaults on a
    /// database error.
    ///
    /// The Matrix command path has no way to surface a settings lookup failure
    /// that would not also destroy the reply the user actually asked for, so a
    /// failed read renders in UTC/USD and logs, rather than aborting.
    pub async fn get_or_default(pool: &DbPool, owner_id: &str) -> UserSettings {
        match Self::get(pool, owner_id).await {
            Ok(settings) => settings,
            Err(err) => {
                tracing::warn!(
                    "Falling back to default settings for {}: {:?}",
                    owner_id,
                    err
                );
                UserSettings::default()
            }
        }
    }

    /// Writes preferences, creating the row if it does not exist.
    pub async fn upsert(
        pool: &DbPool,
        owner_id: &str,
        settings: &UserSettings,
    ) -> Result<UserSettings, sqlx::Error> {
        crate::db_execute!(pool, "INSERT INTO user_settings (owner_id, timezone, currency, dev_room_id, max_history_turns, briefing_enabled, briefing_time, tts_voice, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT(owner_id) DO UPDATE SET \
                timezone = excluded.timezone, \
                currency = excluded.currency, \
                dev_room_id = excluded.dev_room_id, \
                max_history_turns = excluded.max_history_turns, \
                briefing_enabled = excluded.briefing_enabled, \
                briefing_time = excluded.briefing_time, \
                tts_voice = excluded.tts_voice, \
                updated_at = CURRENT_TIMESTAMP", owner_id, &settings.timezone, &settings.currency, &settings.dev_room_id, settings.max_history_turns, settings.briefing_enabled, &settings.briefing_time, &settings.tts_voice)?;

        Self::get(pool, owner_id).await
    }

    /// Retrieves the last date string `YYYY-MM-DD` when the morning briefing was delivered.
    pub async fn get_last_briefing_date(pool: &DbPool, owner_id: &str) -> Option<String> {
        crate::db_scalar_optional!(
            pool,
            String,
            "SELECT last_briefing_date FROM user_settings WHERE owner_id = ?",
            owner_id
        )
        .ok()
        .flatten()
    }

    /// Updates the last briefing date for an owner.
    pub async fn update_last_briefing_date(
        pool: &DbPool,
        owner_id: &str,
        date_str: &str,
    ) -> Result<(), sqlx::Error> {
        crate::db_execute!(
            pool,
            "INSERT INTO user_settings (owner_id, last_briefing_date, updated_at) \
             VALUES (?, ?, CURRENT_TIMESTAMP) \
             ON CONFLICT(owner_id) DO UPDATE SET \
                last_briefing_date = excluded.last_briefing_date, \
                updated_at = CURRENT_TIMESTAMP",
            owner_id,
            date_str
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema::run_migrations;

    async fn pool() -> DbPool {
        let pool = crate::db::pool::test_pool().await;
        run_migrations(&pool).await.unwrap();
        pool
    }

    /// Every user starts without a row, so this is the common path, not an edge.
    #[tokio::test]
    async fn test_a_user_with_no_row_gets_the_defaults() {
        let pool = pool().await;
        let settings = SettingsRepository::get(&pool, "@evan:mbm.one")
            .await
            .unwrap();
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
                briefing_enabled: true,
                briefing_time: "07:30".to_string(),
                tts_voice: Some("alloy".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(created.timezone, "Asia/Shanghai");
        assert_eq!(created.currency, "CNY");
        assert_eq!(created.dev_room_id.as_deref(), Some("!dev:mbm.one"));
        assert_eq!(created.max_history_turns, 15);
        assert!(created.briefing_enabled);
        assert_eq!(created.briefing_time, "07:30");
        assert_eq!(created.tts_voice.as_deref(), Some("alloy"));

        let updated = SettingsRepository::upsert(
            &pool,
            "@evan:mbm.one",
            &UserSettings {
                timezone: "Europe/Berlin".to_string(),
                currency: "EUR".to_string(),
                dev_room_id: None,
                max_history_turns: 20,
                briefing_enabled: false,
                briefing_time: "08:00".to_string(),
                tts_voice: Some("21m00Tcm4TlvDq8ikWAM".to_string()),
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.timezone, "Europe/Berlin");
        assert_eq!(updated.dev_room_id, None);
        assert_eq!(updated.max_history_turns, 20);
        assert!(!updated.briefing_enabled);
        assert_eq!(updated.briefing_time, "08:00");
        assert_eq!(updated.tts_voice.as_deref(), Some("21m00Tcm4TlvDq8ikWAM"));

        // The upsert must update the existing row, never accumulate rows.
        let count: i64 =
            crate::db_scalar_one!(&pool, _, "SELECT COUNT(*) FROM user_settings").unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_last_briefing_date_tracking() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        assert_eq!(
            SettingsRepository::get_last_briefing_date(&pool, owner).await,
            None
        );

        SettingsRepository::update_last_briefing_date(&pool, owner, "2026-09-18")
            .await
            .unwrap();

        assert_eq!(
            SettingsRepository::get_last_briefing_date(&pool, owner).await,
            Some("2026-09-18".to_string())
        );
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
                briefing_enabled: true,
                briefing_time: "07:00".to_string(),
                tts_voice: None,
            },
        )
        .await
        .unwrap();

        // The two identity spaces share a display name but never an owner_id.
        let other = SettingsRepository::get(&pool, "@web-evan:api")
            .await
            .unwrap();
        assert_eq!(other, UserSettings::default());
    }

    /// Save a voice preference, then read it back through a fresh `get` — the
    /// same round trip `!settings voice` and the web UI both rely on.
    #[tokio::test]
    async fn test_tts_voice_round_trips() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        let settings = UserSettings {
            tts_voice: Some("55".to_string()),
            ..UserSettings::default()
        };
        let saved = SettingsRepository::upsert(&pool, owner, &settings)
            .await
            .unwrap();
        assert_eq!(saved.tts_voice.as_deref(), Some("55"));

        let read_back = SettingsRepository::get(&pool, owner).await.unwrap();
        assert_eq!(read_back.tts_voice.as_deref(), Some("55"));
    }

    /// Speaker 0 is a real, selectable voice, not a stand-in for "unset" — it
    /// must round-trip as `Some("0")`, never collapse to `None` on the way
    /// through the database. This is the case the earlier `i64`-with-a-
    /// sentinel design got wrong: saving 0 must not read back as "no
    /// preference".
    #[tokio::test]
    async fn test_saving_speaker_zero_round_trips_as_zero_not_unset() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        let settings = UserSettings {
            tts_voice: Some("0".to_string()),
            ..UserSettings::default()
        };
        let saved = SettingsRepository::upsert(&pool, owner, &settings)
            .await
            .unwrap();
        assert_eq!(saved.tts_voice.as_deref(), Some("0"));

        let read_back = SettingsRepository::get(&pool, owner).await.unwrap();
        assert_eq!(read_back.tts_voice.as_deref(), Some("0"));
    }

    /// A row saved before this column existed must read as `None` ("no
    /// preference") rather than failing — the column is nullable and
    /// `run_migrations`'s best-effort `ALTER TABLE` adds it with no default,
    /// so SQLite backfills existing rows with NULL.
    #[tokio::test]
    async fn test_a_row_that_predates_the_column_reads_as_unset() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        // Written without ever mentioning tts_voice, exactly like a row an
        // older binary would have created.
        crate::db_execute!(
            &pool,
            "INSERT INTO user_settings (owner_id, timezone, currency) VALUES (?, 'UTC', 'USD')",
            owner
        )
        .unwrap();

        let settings = SettingsRepository::get(&pool, owner).await.unwrap();
        assert_eq!(settings.tts_voice, None);
    }
}
