use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    // Per-user display preferences, keyed by owner_id.
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS user_settings (
            owner_id TEXT PRIMARY KEY,
            timezone TEXT NOT NULL DEFAULT 'UTC',
            currency TEXT NOT NULL DEFAULT 'USD',
            dev_room_id TEXT,
            max_history_turns BIGINT NOT NULL DEFAULT 10,
            briefing_enabled BIGINT NOT NULL DEFAULT 1,
            briefing_time TEXT NOT NULL DEFAULT '07:00',
            last_briefing_date TEXT,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    let _ = db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN dev_room_id TEXT;"
    );

    let _ = db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN max_history_turns BIGINT NOT NULL DEFAULT 10;"
    );

    let _ = db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN briefing_enabled BIGINT NOT NULL DEFAULT 1;"
    );

    let _ = db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN briefing_time TEXT NOT NULL DEFAULT '07:00';"
    );

    let _ = db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN last_briefing_date TEXT;"
    );

    // Nullable, unlike the other columns here: NULL means "no preference, use
    // TTS_SPEAKER_ID", and 0 is a real, selectable speaker index rather than a
    // stand-in for "unset" (see `modules::settings::models::UserSettings`).
    //
    // SQLite cannot alter a column's constraints in place, and this ALTER is
    // best-effort like its neighbours, so a dev database that already ran an
    // earlier build of this branch (which added this column as
    // `NOT NULL DEFAULT 0`) keeps that shape. On such a database every row's
    // `tts_speaker_id` reads back as `Some(0)` rather than `None` — the
    // operator default silently stops applying for that row until the user
    // re-picks a voice. Acceptable on an unreleased branch; drop the affected
    // table (or the whole dev database) if this bites.
    let _ = db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN tts_speaker_id BIGINT;"
    );

    Ok(())
}
