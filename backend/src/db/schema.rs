use sqlx::SqlitePool;
use tracing::info;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    info!("Running database table schema initialization...");

    // The ownership column used to be called `sender_mxid`, because it held a
    // Matrix user ID. It now holds an `accounts.id`, so the old name actively
    // lies about what is in it — and a column whose name contradicts its
    // contents is exactly how the two-identity-space bug survived as long as it
    // did. Renamed first, before anything below reads or indexes the column.
    //
    // Best-effort per the convention in this file: on a fresh database the
    // table does not exist yet, and on an already-migrated one the column is
    // already called `owner_id`. Both raise an error that means "nothing to do".
    // SQLite rewrites index definitions to follow a renamed column, so the
    // `CREATE INDEX IF NOT EXISTS` calls further down stay no-ops.
    for table in ["todos", "reminders", "transactions", "recurring_bills"] {
        let _ = sqlx::query(&format!(
            "ALTER TABLE {table} RENAME COLUMN sender_mxid TO owner_id;"
        ))
        .execute(pool)
        .await;
    }

    crate::db::schema_auth::create_tables(pool).await?;
    crate::db::schema_webhooks::create_tables(pool).await?;
    crate::db::schema_ai::create_tables(pool).await?;
    crate::db::schema_portfolio::create_tables(pool).await?;
    crate::db::schema_footprints::create_tables(pool).await?;
    crate::db::schema_habits::create_tables(pool).await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            text TEXT NOT NULL,
            priority TEXT NOT NULL DEFAULT 'medium',
            due_at TEXT,
            completed BOOLEAN NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS reminders (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            owner_id TEXT NOT NULL,
            sender_name TEXT NOT NULL,
            message TEXT NOT NULL,
            trigger_at TIMESTAMP NOT NULL,
            recurrence TEXT NOT NULL DEFAULT 'none',
            status TEXT NOT NULL DEFAULT 'pending',
            retry_count INTEGER NOT NULL DEFAULT 0,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS web_users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT UNIQUE NOT NULL,
            password_hash TEXT NOT NULL,
            created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Per-user display preferences, keyed by the same `owner_id` as every other
    // ownership check. A user with no row uses the defaults, so rows appear only
    // when a preference is actually changed.
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS user_settings (
            owner_id TEXT PRIMARY KEY,
            timezone TEXT NOT NULL DEFAULT 'UTC',
            currency TEXT NOT NULL DEFAULT 'USD',
            updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )
    .execute(pool)
    .await?;

    // Investment holdings. One row per (owner, symbol) — `quantity_minor` and
    // `avg_cost_minor` are integers so weighted-average cost math never touches
    // a float; `currency_exponent` is stored per row for the same reason the
    // ledger stores it: a later change to the currency table cannot rewrite
    // what an existing row means.
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

    // Cash available to buy with, one row per (owner, currency). No CHECK
    // constraint on `balance_minor` — nothing else in this schema uses one;
    // non-negativity is enforced in `PortfolioRepository`, the same
    // app-level-only convention `quantity_minor`/`avg_cost_minor` already use.
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

    sqlx::query(
        r#"
        CREATE TRIGGER IF NOT EXISTS prevent_portfolio_audit_update
        BEFORE UPDATE ON portfolio_audit_log
        BEGIN
            SELECT RAISE(ABORT, 'Audit log entries are immutable and cannot be updated');
        END;
        "#,
    )
    .execute(pool)
    .await?;

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

    // Composite indexes for performance optimization
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_status_trigger ON reminders(status, trigger_at);"
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(owner_id, room_id);"
    )
    .execute(pool)
    .await?;

    // Migration helper for existing sqlite DBs missing retry_count column
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;")
        .execute(pool)
        .await;

    // How long before `due_at` to fire a reminder. Kept on the todo rather than
    // derived from the linked reminder so the web UI can read the current
    // setting back without reverse-engineering it from a timestamp difference.
    let _ = sqlx::query("ALTER TABLE todos ADD COLUMN remind_before_minutes INTEGER;")
        .execute(pool)
        .await;

    // Links a reminder back to the row whose due date scheduled it, so the
    // reminder can be rescheduled or cancelled when that row changes.
    //
    // This started as a bare `todo_id`. Bills want the same lead-time reminder,
    // and a second nullable foreign key per module does not scale, so the link
    // is now `(source_kind, source_id)`: one set of repository functions serves
    // every module that owns a due date.
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN todo_id INTEGER;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN source_kind TEXT;")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN source_id INTEGER;")
        .execute(pool)
        .await;

    // Carry existing todo links over. Guarded on `source_id IS NULL` so a
    // re-run cannot overwrite a link that has since moved.
    let _ = sqlx::query(
        "UPDATE reminders SET source_kind = 'todo', source_id = todo_id \
         WHERE todo_id IS NOT NULL AND source_id IS NULL;",
    )
    .execute(pool)
    .await;

    let _ = sqlx::query("DROP INDEX IF EXISTS idx_reminders_todo;")
        .execute(pool)
        .await;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_reminders_source ON reminders(source_kind, source_id);",
    )
    .execute(pool)
    .await?;

    // Migration helpers for recurring_bills columns on existing SQLite DBs
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN currency_exponent INTEGER NOT NULL DEFAULT 2;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN frequency TEXT NOT NULL DEFAULT 'monthly';").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN due_day INTEGER;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN next_due_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN total_installments INTEGER;").execute(pool).await;
    // Lead-time reminder, mirroring the todo columns above: minutes before
    // `next_due_at`, and where to deliver it when the bill has no room of its
    // own (the `web` sentinel).
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN remind_before_minutes INTEGER;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN remind_room_id TEXT;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN paid_installments INTEGER NOT NULL DEFAULT 0;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN auto_pay BOOLEAN NOT NULL DEFAULT 0;").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN status TEXT NOT NULL DEFAULT 'active';").execute(pool).await;
    let _ = sqlx::query("ALTER TABLE recurring_bills ADD COLUMN notes TEXT;").execute(pool).await;

    // `due_at` used to hold the rendered display string (`2026-12-31 23:59 UTC`),
    // which cannot be compared or subtracted from. Rewrite legacy rows into the
    // timestamp format sqlx reads back into `DateTime<Utc>`. The `strftime`
    // guard leaves anything unparseable alone rather than nulling the column.
    let _ = sqlx::query(
        r#"
        UPDATE todos
        SET due_at = strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', ''))
        WHERE due_at LIKE '% UTC'
          AND strftime('%Y-%m-%d %H:%M:%S+00:00', replace(due_at, ' UTC', '')) IS NOT NULL;
        "#,
    )
    .execute(pool)
    .await;

    // Clean up legacy matrix.org room_id placeholders
    let _ = sqlx::query("UPDATE reminders SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;
    let _ = sqlx::query("UPDATE todos SET room_id = 'web' WHERE room_id LIKE '%matrix.org%';")
        .execute(pool)
        .await;

    // Ties an existing password login to its account. Phase 2 deletes
    // `web_users` entirely and moves the credential onto `accounts`; this
    // column is what keeps password login working in the meantime.
    let _ = sqlx::query("ALTER TABLE web_users ADD COLUMN account_id TEXT;")
        .execute(pool)
        .await;

    let _ = sqlx::query("ALTER TABLE portfolio_audit_log ADD COLUMN prev_hash TEXT NOT NULL DEFAULT '0000000000000000000000000000000000000000000000000000000000000000';")
        .execute(pool)
        .await;
    let _ = sqlx::query("ALTER TABLE portfolio_audit_log ADD COLUMN entry_hash TEXT NOT NULL DEFAULT '';")
        .execute(pool)
        .await;

    // One-shot data conversions, kept out of the DDL above because they are a
    // different job: this function describes the shape of the database, and
    // `backfill` rewrites what is already in it.
    crate::db::backfill::run(pool).await?;

    info!("Database migrations applied successfully.");
    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn table_names(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn test_creates_every_table() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let tables = table_names(&pool).await;
        for expected in [
            "finance_categories",
            "portfolio_audit_log",
            "portfolio_cash_balances",
            "portfolio_holdings",
            "recurring_bills",
            "reminders",
            "todos",
            "transactions",
            "user_settings",
            "web_users",
        ] {
            assert!(tables.contains(&expected.to_string()), "missing table {expected}");
        }
    }

    /// This is the only migration mechanism there is, and it runs on every boot.
    /// It must be safe to apply repeatedly to an already-migrated database.
    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();

        run_migrations(&pool).await.unwrap();
        let first = table_names(&pool).await;

        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let third = table_names(&pool).await;

        assert_eq!(first, third, "re-running migrations must not change the schema");
    }

    /// Re-running migrations must not disturb rows that already exist.
    #[tokio::test]
    async fn test_migrations_preserve_existing_data() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', '@a:x', 'a', 'keep me', 'medium', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        run_migrations(&pool).await.unwrap();

        let text: String = sqlx::query_scalar("SELECT text FROM todos")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(text, "keep me");
    }

    /// The unique index is what stops a user ending up with two categories of
    /// the same name; the repository relies on it rather than a read-then-write.
    #[tokio::test]
    async fn test_category_name_uniqueness_is_enforced_by_the_schema() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')")
            .execute(&pool)
            .await
            .unwrap();

        let duplicate =
            sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')")
                .execute(&pool)
                .await;
        assert!(duplicate.is_err(), "a duplicate name must be rejected by the index");

        // A different owner may reuse the name.
        sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@b:x', 'food')")
            .execute(&pool)
            .await
            .unwrap();
    }
}
