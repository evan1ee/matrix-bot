use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    // Per-user display preferences, keyed by owner_id.
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS user_settings (
            owner_id TEXT PRIMARY KEY,
            timezone TEXT NOT NULL DEFAULT 'UTC',
            currency TEXT NOT NULL DEFAULT 'USD',
            dev_room_id TEXT,
            max_history_turns BIGINT NOT NULL DEFAULT 10,
            briefing_enabled BOOLEAN NOT NULL DEFAULT TRUE,
            briefing_time TEXT NOT NULL DEFAULT '07:00',
            last_briefing_date TEXT,
            updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN dev_room_id TEXT;"
    );

    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN max_history_turns BIGINT NOT NULL DEFAULT 10;"
    );

    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN briefing_enabled BOOLEAN NOT NULL DEFAULT TRUE;"
    );

    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN briefing_time TEXT NOT NULL DEFAULT '07:00';"
    );

    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE user_settings ADD COLUMN last_briefing_date TEXT;"
    );

    // Nullable: NULL means "no preference, use TTS_VOICE". A TEXT column
    // because a voice is whatever the configured provider calls one ("0" for
    // Kokoro, "alloy" for OpenAI, an id for ElevenLabs).
    let _ = crate::db_ddl!(pool, "ALTER TABLE user_settings ADD COLUMN tts_voice TEXT;");

    // The integer column this replaced. Dropped rather than migrated: the
    // project does not carry compatibility paths, and a saved Kokoro index
    // would be meaningless to any other provider. Best-effort like every
    // ALTER here: a fresh database never had it.
    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE user_settings DROP COLUMN tts_speaker_id;"
    );

    Ok(())
}
