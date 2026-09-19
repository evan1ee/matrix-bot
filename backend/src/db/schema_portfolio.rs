use sqlx::SqlitePool;

/// The statement that creates the audit-log immutability trigger.
///
/// Public because `modules::portfolio::audit::absorb_ledger` has to drop and
/// restore this trigger to move a ledger between owners, and two copies of the
/// text would drift.
pub const AUDIT_UPDATE_TRIGGER: &str = r#"
        CREATE TRIGGER IF NOT EXISTS prevent_portfolio_audit_update
        BEFORE UPDATE ON portfolio_audit_log
        BEGIN
            SELECT RAISE(ABORT, 'Audit log entries are immutable and cannot be updated');
        END;
        "#;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    // Investment holdings. One row per (owner, symbol)
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_holdings (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            quantity_minor INTEGER NOT NULL,
            avg_cost_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            current_price_minor INTEGER,
            price_updated_at TIMESTAMP,
            note TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_holdings_owner_symbol \
         ON portfolio_holdings(owner_id, symbol);",
    )
    .execute(pool)
    .await?;

    // Cash available to buy with, one row per (owner, currency).
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_cash_balances (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            balance_minor INTEGER NOT NULL DEFAULT 0,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_cash_owner_currency \
         ON portfolio_cash_balances(owner_id, currency);",
    )
    .execute(pool)
    .await?;

    // Append-only, cryptographically linked immutable audit log for all portfolio activity.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_audit_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            action TEXT NOT NULL,
            symbol TEXT,
            quantity_minor INTEGER,
            price_minor INTEGER,
            amount_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            realized_pl_minor INTEGER,
            note TEXT,
            prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000',
            entry_hash TEXT NOT NULL DEFAULT '',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_portfolio_audit_log_owner \
         ON portfolio_audit_log(owner_id, created_at DESC);",
    )
    .execute(pool)
    .await?;

    sqlx::query(AUDIT_UPDATE_TRIGGER).execute(pool).await?;

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS prevent_portfolio_audit_delete
        BEFORE DELETE ON portfolio_audit_log
        BEGIN
            SELECT RAISE(ABORT, 'Audit log entries cannot be deleted');
        END;
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::query("ALTER TABLE portfolio_audit_log ADD COLUMN prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';")
        .execute(pool)
        .await;
    let _ = sqlx::query(
        "ALTER TABLE portfolio_audit_log ADD COLUMN entry_hash TEXT NOT NULL DEFAULT '';",
    )
    .execute(pool)
    .await;

    // Portfolio Price & Volatility Alerts
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_price_alerts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            target_price_minor INTEGER NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            condition TEXT NOT NULL,
            is_triggered BOOLEAN NOT NULL DEFAULT 0,
            triggered_at TIMESTAMP,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_price_alerts_owner \
         ON portfolio_price_alerts(owner_id, is_triggered);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_price_alerts_symbol \
         ON portfolio_price_alerts(symbol, is_triggered);",
    )
    .execute(pool)
    .await?;

    // Daily Portfolio Performance & Net Worth Snapshots
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_daily_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_id TEXT NOT NULL,
            snapshot_date TEXT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent INTEGER NOT NULL DEFAULT 2,
            holdings_value_minor INTEGER NOT NULL,
            cash_minor INTEGER NOT NULL,
            total_net_worth_minor INTEGER NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(owner_id, snapshot_date, currency)
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_portfolio_snapshots_lookup \
         ON portfolio_daily_snapshots(owner_id, currency, snapshot_date ASC);",
    )
    .execute(pool)
    .await?;

    Ok(())
}
