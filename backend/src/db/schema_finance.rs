use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Per-user spending categories. Names are unique per owner, not globally.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS finance_categories (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_finance_categories_owner_name \
         ON finance_categories(owner_id, name);",
    )
    .execute(pool)
    .await?;

    // The ledger. `amount_minor` is always positive and `direction` carries the
    // sign, so a row cannot contradict itself; `currency_exponent` is stored per
    // row so a later change to the currency table cannot rewrite history.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS transactions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            direction TEXT NOT NULL,
            amount_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            category_id INTEGER,
            note TEXT,
            source_kind TEXT,
            source_id INTEGER,
            occurred_at TIMESTAMP NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_transactions_owner_occurred \
         ON transactions(owner_id, occurred_at DESC);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);",
    )
    .execute(pool)
    .await?;

    // Link transactions to external originating sources (e.g. recurring_bill, portfolio_cash).
    let _ = sqlx::query("ALTER TABLE transactions ADD COLUMN source_kind TEXT;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE transactions ADD COLUMN source_id INTEGER;")
        .execute(pool)
        .await;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_transactions_source ON transactions(source_kind, source_id);",
    )
    .execute(pool)
    .await?;

    // Public sharing tokens for fiscal transparency portal
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS finance_shares (
            owner_id TEXT PRIMARY KEY,
            token TEXT NOT NULL UNIQUE,
            is_enabled BOOLEAN NOT NULL DEFAULT 1,
            title TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_finance_shares_token ON finance_shares(token);")
        .execute(pool)
        .await?;

    Ok(())
}
