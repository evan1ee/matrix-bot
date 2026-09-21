use crate::db::DbPool;
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS recurring_bills (
            id {{PK}},
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            name TEXT NOT NULL,
            category TEXT NOT NULL,
            amount_minor BIGINT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            frequency TEXT NOT NULL DEFAULT 'monthly',
            due_day BIGINT,
            next_due_at TIMESTAMP NOT NULL,
            total_installments BIGINT,
            paid_installments BIGINT NOT NULL DEFAULT 0,
            auto_pay BOOLEAN NOT NULL DEFAULT FALSE,
            status TEXT NOT NULL DEFAULT 'active',
            notes TEXT,
            remind_before_minutes BIGINT,
            remind_room_id TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_recurring_bills_owner ON recurring_bills(owner_id, status);")?;

    db_ddl!(pool, "CREATE INDEX IF NOT EXISTS idx_recurring_bills_due ON recurring_bills(owner_id, next_due_at);")?;

    // Migration helpers for recurring_bills columns on existing SQLite DBs
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN currency_exponent BIGINT NOT NULL DEFAULT 2;"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN frequency TEXT NOT NULL DEFAULT 'monthly';"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN due_day BIGINT;"
    );
    let _ = db_ddl!(pool, "ALTER TABLE recurring_bills ADD COLUMN next_due_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP;");
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN total_installments BIGINT;"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN remind_before_minutes BIGINT;"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN remind_room_id TEXT;"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN paid_installments BIGINT NOT NULL DEFAULT 0;"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN auto_pay BOOLEAN NOT NULL DEFAULT FALSE;"
    );
    let _ = db_ddl!(
        pool,
        "ALTER TABLE recurring_bills ADD COLUMN status TEXT NOT NULL DEFAULT 'active';"
    );
    let _ = db_ddl!(pool, "ALTER TABLE recurring_bills ADD COLUMN notes TEXT;");

    Ok(())
}
