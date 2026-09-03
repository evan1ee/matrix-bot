use sqlx::SqlitePool;

pub async fn create_tables(pool: &SqlitePool) -> Result<(), sqlx::Error> {
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
