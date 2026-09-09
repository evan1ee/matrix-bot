use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS footprints (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            title TEXT NOT NULL,
            latitude REAL NOT NULL,
            longitude REAL NOT NULL,
            country TEXT,
            city TEXT,
            address TEXT,
            arrival_date TEXT NOT NULL,
            departure_date TEXT,
            stay_duration_days INTEGER NOT NULL DEFAULT 1,
            spend_amount_minor INTEGER NOT NULL DEFAULT 0,
            spend_currency TEXT NOT NULL DEFAULT 'USD',
            spend_currency_exponent INTEGER NOT NULL DEFAULT 2,
            rating INTEGER NOT NULL DEFAULT 5,
            category TEXT NOT NULL DEFAULT 'travel',
            note TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_footprints_owner_arrival \
         ON footprints(owner_id, arrival_date DESC);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_footprints_country \
         ON footprints(owner_id, country);",
    )
    .execute(pool)
    .await?;

    Ok(())
}
