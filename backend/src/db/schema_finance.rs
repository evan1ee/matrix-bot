use crate::db::DbPool;
pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    // Per-user spending categories. Names are unique per owner, not globally.
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS finance_categories (
            id {{PK}},
            owner_id TEXT NOT NULL,
            name TEXT NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_finance_categories_owner_name \
         ON finance_categories(owner_id, name);"
    )?;

    // The ledger. `amount_minor` is always positive and `direction` carries the
    // sign, so a row cannot contradict itself; `currency_exponent` is stored per
    // row so a later change to the currency table cannot rewrite history.
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS transactions (
            id {{PK}},
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            direction TEXT NOT NULL,
            amount_minor BIGINT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            category_id BIGINT,
            note TEXT,
            source_kind TEXT,
            source_id BIGINT,
            occurred_at TIMESTAMPTZ NOT NULL,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_transactions_owner_occurred \
         ON transactions(owner_id, occurred_at DESC);"
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);"
    )?;

    // Link transactions to external originating sources (e.g. recurring_bill, portfolio_cash).
    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE transactions ADD COLUMN source_kind TEXT;"
    );
    let _ = crate::db_ddl!(
        pool,
        "ALTER TABLE transactions ADD COLUMN source_id BIGINT;"
    );
    crate::db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_transactions_source ON transactions(source_kind, source_id);")?;

    // Public sharing tokens for fiscal transparency portal
    crate::db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS finance_shares (
            owner_id TEXT PRIMARY KEY,
            token TEXT NOT NULL UNIQUE,
            is_enabled BOOLEAN NOT NULL DEFAULT TRUE,
            title TEXT,
            created_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMPTZ DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    crate::db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_finance_shares_token ON finance_shares(token);"
    )?;

    Ok(())
}
