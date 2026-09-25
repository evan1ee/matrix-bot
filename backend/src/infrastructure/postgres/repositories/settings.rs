//! `user_settings`: one row per owner, holding display preferences. The only
//! SQL for settings; every function takes any `PgExecutor`, so the service
//! can run it on the pool or inside its own transaction.

use sqlx::PgExecutor;

use crate::domain::settings::UserSettings;

/// `user_settings` as stored. Private: rows become the domain type at this
/// edge. `From`, not `TryFrom` — every stored row is a valid `UserSettings`
/// (an unparseable `timezone` degrades to UTC in `UserSettings::tz`, rather
/// than being rejected on read).
struct UserSettingsRow {
    timezone: String,
    currency: String,
    dev_room_id: Option<String>,
    max_history_turns: i64,
    briefing_enabled: bool,
    briefing_time: String,
    tts_voice: Option<String>,
}

impl From<UserSettingsRow> for UserSettings {
    fn from(row: UserSettingsRow) -> Self {
        UserSettings {
            timezone: row.timezone,
            currency: row.currency,
            dev_room_id: row.dev_room_id,
            max_history_turns: row.max_history_turns,
            briefing_enabled: row.briefing_enabled,
            briefing_time: row.briefing_time,
            tts_voice: row.tts_voice,
        }
    }
}

/// The caller's stored settings, or `None` if they have never saved any —
/// the ordinary case, since a row is written only when a user changes a
/// preference. Callers apply the "missing row means defaults" rule
/// themselves (`domain::settings::UserSettings::default`).
pub async fn get(
    conn: impl PgExecutor<'_>,
    owner_id: &str,
) -> Result<Option<UserSettings>, sqlx::Error> {
    let row = sqlx::query_as!(
        UserSettingsRow,
        "SELECT timezone, currency, dev_room_id, max_history_turns, briefing_enabled, \
         briefing_time, tts_voice FROM user_settings WHERE owner_id = $1",
        owner_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(UserSettings::from))
}

/// Like [`get`], but locks the row for the rest of the caller's transaction
/// (`SELECT ... FOR UPDATE`) so a concurrent read-modify-write for the same
/// owner blocks instead of racing. `application::settings::SettingsService::modify`
/// is the only caller; a missing row locks nothing (there is nothing yet to
/// lock), which is fine — the `INSERT ... ON CONFLICT` in `upsert` is what
/// makes the first write for a new owner atomic.
pub async fn get_for_update(
    conn: impl PgExecutor<'_>,
    owner_id: &str,
) -> Result<Option<UserSettings>, sqlx::Error> {
    let row = sqlx::query_as!(
        UserSettingsRow,
        "SELECT timezone, currency, dev_room_id, max_history_turns, briefing_enabled, \
         briefing_time, tts_voice FROM user_settings WHERE owner_id = $1 FOR UPDATE",
        owner_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.map(UserSettings::from))
}

/// Writes preferences, creating the row if it does not exist.
pub async fn upsert(
    conn: impl PgExecutor<'_>,
    owner_id: &str,
    settings: &UserSettings,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO user_settings \
            (owner_id, timezone, currency, dev_room_id, max_history_turns, \
             briefing_enabled, briefing_time, tts_voice, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, CURRENT_TIMESTAMP) \
         ON CONFLICT (owner_id) DO UPDATE SET \
            timezone = excluded.timezone, \
            currency = excluded.currency, \
            dev_room_id = excluded.dev_room_id, \
            max_history_turns = excluded.max_history_turns, \
            briefing_enabled = excluded.briefing_enabled, \
            briefing_time = excluded.briefing_time, \
            tts_voice = excluded.tts_voice, \
            updated_at = CURRENT_TIMESTAMP",
        owner_id,
        settings.timezone,
        settings.currency,
        settings.dev_room_id,
        settings.max_history_turns,
        settings.briefing_enabled,
        settings.briefing_time,
        settings.tts_voice,
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// The last date string `YYYY-MM-DD` the morning briefing was delivered.
pub async fn get_last_briefing_date(
    conn: impl PgExecutor<'_>,
    owner_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT last_briefing_date FROM user_settings WHERE owner_id = $1",
        owner_id
    )
    .fetch_optional(conn)
    .await?;
    Ok(row.and_then(|r| r.last_briefing_date))
}

/// Updates the last briefing date for an owner.
pub async fn update_last_briefing_date(
    conn: impl PgExecutor<'_>,
    owner_id: &str,
    date_str: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "INSERT INTO user_settings (owner_id, last_briefing_date, updated_at) \
         VALUES ($1, $2, CURRENT_TIMESTAMP) \
         ON CONFLICT (owner_id) DO UPDATE SET \
            last_briefing_date = excluded.last_briefing_date, \
            updated_at = CURRENT_TIMESTAMP",
        owner_id,
        date_str
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> sqlx::PgPool {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        pool
    }

    fn settings(timezone: &str, currency: &str) -> UserSettings {
        UserSettings {
            timezone: timezone.to_string(),
            currency: currency.to_string(),
            ..UserSettings::default()
        }
    }

    /// Every user starts without a row, so this is the common path, not an edge.
    #[tokio::test]
    async fn a_user_with_no_row_has_no_stored_settings() {
        let pool = pool().await;
        assert_eq!(get(&pool, "@evan:mbm.one").await.unwrap(), None);
    }

    #[tokio::test]
    async fn upsert_creates_then_updates_in_place() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        upsert(
            &pool,
            owner,
            &UserSettings {
                dev_room_id: Some("!dev:mbm.one".to_string()),
                max_history_turns: 15,
                briefing_time: "07:30".to_string(),
                tts_voice: Some("alloy".to_string()),
                ..settings("Asia/Shanghai", "CNY")
            },
        )
        .await
        .unwrap();
        let created = get(&pool, owner).await.unwrap().unwrap();
        assert_eq!(created.timezone, "Asia/Shanghai");
        assert_eq!(created.currency, "CNY");
        assert_eq!(created.dev_room_id.as_deref(), Some("!dev:mbm.one"));
        assert_eq!(created.max_history_turns, 15);
        assert_eq!(created.tts_voice.as_deref(), Some("alloy"));

        upsert(
            &pool,
            owner,
            &UserSettings {
                dev_room_id: None,
                max_history_turns: 20,
                briefing_enabled: false,
                ..settings("Europe/Berlin", "EUR")
            },
        )
        .await
        .unwrap();
        let updated = get(&pool, owner).await.unwrap().unwrap();
        assert_eq!(updated.timezone, "Europe/Berlin");
        assert_eq!(updated.dev_room_id, None);
        assert_eq!(updated.max_history_turns, 20);
        assert!(!updated.briefing_enabled);

        let count: i64 = sqlx::query_scalar!("SELECT COUNT(*) AS \"count!\" FROM user_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            count, 1,
            "upsert must update the existing row, never accumulate rows"
        );
    }

    /// Speaker 0 is a real, selectable voice — saving it must not be confused
    /// with clearing the preference (an earlier `i64`-with-sentinel design got
    /// this wrong).
    #[tokio::test]
    async fn saving_speaker_zero_round_trips_as_zero_not_unset() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        upsert(
            &pool,
            owner,
            &UserSettings {
                tts_voice: Some("0".to_string()),
                ..UserSettings::default()
            },
        )
        .await
        .unwrap();

        let read_back = get(&pool, owner).await.unwrap().unwrap();
        assert_eq!(read_back.tts_voice.as_deref(), Some("0"));
    }

    /// `owner_id` is the ownership key everywhere else in the system;
    /// settings must not be an exception.
    #[tokio::test]
    async fn settings_are_isolated_per_owner() {
        let pool = pool().await;
        upsert(&pool, "@evan:mbm.one", &settings("Asia/Shanghai", "CNY"))
            .await
            .unwrap();

        assert_eq!(get(&pool, "@web-evan:api").await.unwrap(), None);
    }

    #[tokio::test]
    async fn last_briefing_date_tracking() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        assert_eq!(get_last_briefing_date(&pool, owner).await.unwrap(), None);

        update_last_briefing_date(&pool, owner, "2026-09-18")
            .await
            .unwrap();

        assert_eq!(
            get_last_briefing_date(&pool, owner).await.unwrap(),
            Some("2026-09-18".to_string())
        );
    }

    /// A row saved before `tts_voice` existed must read as `None` ("no
    /// preference") — the column is nullable and `run_migrations`'s
    /// best-effort `ALTER TABLE` adds it with no default.
    #[tokio::test]
    async fn a_row_that_predates_the_tts_voice_column_reads_as_unset() {
        let pool = pool().await;
        let owner = "@evan:mbm.one";

        sqlx::query!(
            "INSERT INTO user_settings (owner_id, timezone, currency) VALUES ($1, 'UTC', 'USD')",
            owner
        )
        .execute(&pool)
        .await
        .unwrap();

        let settings = get(&pool, owner).await.unwrap().unwrap();
        assert_eq!(settings.tts_voice, None);
    }
}
