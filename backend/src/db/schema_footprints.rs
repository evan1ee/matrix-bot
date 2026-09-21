use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS footprints (
            id {{PK}},
            owner_id TEXT NOT NULL,
            title TEXT NOT NULL,
            latitude REAL NOT NULL,
            longitude REAL NOT NULL,
            country TEXT,
            city TEXT,
            address TEXT,
            arrival_date TEXT NOT NULL,
            departure_date TEXT,
            stay_duration_days BIGINT NOT NULL DEFAULT 1,
            spend_amount_minor BIGINT NOT NULL DEFAULT 0,
            spend_currency TEXT NOT NULL DEFAULT 'USD',
            spend_currency_exponent BIGINT NOT NULL DEFAULT 2,
            rating BIGINT NOT NULL DEFAULT 5,
            category TEXT NOT NULL DEFAULT 'travel',
            note TEXT,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_footprints_owner_arrival \
         ON footprints(owner_id, arrival_date DESC);"
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_footprints_country \
         ON footprints(owner_id, country);"
    )?;

    Ok(())
}
