use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS recurring_bills (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            name TEXT NOT NULL,
            category TEXT NOT NULL,
            amount_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            frequency TEXT NOT NULL DEFAULT 'monthly',
            due_day INTEGER,
            next_due_at TIMESTAMP NOT NULL,
            total_installments INTEGER,
            paid_installments INTEGER NOT NULL DEFAULT 0,
            auto_pay BOOLEAN NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'active',
            notes TEXT,
            remind_before_minutes INTEGER,
            remind_room_id TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recurring_bills_owner ON recurring_bills(owner_id, status);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_recurring_bills_due ON recurring_bills(owner_id, next_due_at);",
    )
    .execute(pool)
    .await?;

    // Migration helpers for recurring_bills columns on existing SQLite DBs
    let _ = sqlx::query(
        "ALTER TABLE recurring_bills ADD COLUMN currency_exponent INTEGER NOT NULL DEFAULT 2;",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "ALTER TABLE recurring_bills ADD COLUMN frequency TEXT NOT NULL DEFAULT 'monthly';",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN due_day INTEGER;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN next_due_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN total_installments INTEGER;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN remind_before_minutes INTEGER;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN remind_room_id TEXT;")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "ALTER TABLE recurring_bills ADD COLUMN paid_installments INTEGER NOT NULL DEFAULT 0;",
    )
    .execute(pool)
    .await;
    let _ =
        sqlx::query("ALTER TABLE recurring_bills ADD COLUMN auto_pay BOOLEAN NOT NULL DEFAULT 0;")
            .execute(pool)
            .await;
    let _ = sqlx::query(
        "ALTER TABLE recurring_bills ADD COLUMN status TEXT NOT NULL DEFAULT 'active';",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN notes TEXT;")
        .execute(pool)
        .await;

    Ok(())
}
