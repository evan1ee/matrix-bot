use crate::db::{ddl, DbPool};
use crate::db_ddl;

pub async fn create_tables(pool: &DbPool) -> Result<(), sqlx::Error> {
    // Investment holdings. One row per (owner, symbol)
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_holdings (
            id {{PK}},
            owner_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            quantity_minor BIGINT NOT NULL,
            avg_cost_minor BIGINT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            current_price_minor BIGINT,
            price_updated_at TIMESTAMP,
            note TEXT,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_holdings_owner_symbol \
         ON portfolio_holdings(owner_id, symbol);"
    )?;

    // Cash available to buy with, one row per (owner, currency).
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_cash_balances (
            id {{PK}},
            owner_id TEXT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            balance_minor BIGINT NOT NULL DEFAULT 0,
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_portfolio_cash_owner_currency \
         ON portfolio_cash_balances(owner_id, currency);"
    )?;

    // Append-only, cryptographically linked immutable audit log for all portfolio activity.
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_audit_log (
            id {{PK}},
            owner_id TEXT NOT NULL,
            action TEXT NOT NULL,
            symbol TEXT,
            quantity_minor BIGINT,
            price_minor BIGINT,
            amount_minor BIGINT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            realized_pl_minor BIGINT,
            note TEXT,
            prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000',
            entry_hash TEXT NOT NULL DEFAULT '',
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_portfolio_audit_log_owner \
         ON portfolio_audit_log(owner_id, created_at DESC);"
    )?;

    // Written per dialect rather than translated: PostgreSQL has no
    // CREATE TRIGGER IF NOT EXISTS and needs a plpgsql function behind each
    // guard, so it emits three statements where SQLite emits one.
    for statement in ddl::audit_log_immutability(pool.dialect()) {
        db_ddl!(pool, &statement)?;
    }

    let _ = db_ddl!(pool, "ALTER TABLE portfolio_audit_log ADD COLUMN prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';");
    let _ = db_ddl!(
        pool,
        "ALTER TABLE portfolio_audit_log ADD COLUMN entry_hash TEXT NOT NULL DEFAULT '';"
    );

    // Portfolio Price & Volatility Alerts
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_price_alerts (
            id {{PK}},
            owner_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            target_price_minor BIGINT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            condition TEXT NOT NULL,
            is_triggered BOOLEAN NOT NULL DEFAULT FALSE,
            triggered_at TIMESTAMP,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_price_alerts_owner \
         ON portfolio_price_alerts(owner_id, is_triggered);"
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_price_alerts_symbol \
         ON portfolio_price_alerts(symbol, is_triggered);"
    )?;

    // Daily Portfolio Performance & Net Worth Snapshots
    db_ddl!(
        pool,
        r#"
        CREATE TABLE IF NOT EXISTS portfolio_daily_snapshots (
            id {{PK}},
            owner_id TEXT NOT NULL,
            snapshot_date TEXT NOT NULL,
            currency TEXT NOT NULL,
            currency_exponent BIGINT NOT NULL DEFAULT 2,
            holdings_value_minor BIGINT NOT NULL,
            cash_minor BIGINT NOT NULL,
            total_net_worth_minor BIGINT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(owner_id, snapshot_date, currency)
        );
        "#
    )?;

    db_ddl!(
        pool,
        "CREATE INDEX IF NOT EXISTS idx_portfolio_snapshots_lookup \
         ON portfolio_daily_snapshots(owner_id, currency, snapshot_date ASC);"
    )?;

    Ok(())
}
