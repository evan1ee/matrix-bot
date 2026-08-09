use sqlx::SqlitePool;
use tracing::info;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    info!("Running database table schema initialization...");

    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS todos (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            room_id TEXT NOT NULL,
            sender_mxid TEXT NOT NULL,
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
            sender_mxid TEXT NOT NULL,
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
            sender_mxid TEXT NOT NULL,
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
         ON transactions(sender_mxid, occurred_at DESC);",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_transactions_category ON transactions(category_id);",
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
        "CREATE INDEX IF NOT EXISTS idx_todos_sender_room ON todos(sender_mxid, room_id);"
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

    // Links a reminder back to the todo whose due date scheduled it, so the
    // reminder can be rescheduled or cancelled when that todo changes.
    let _ = sqlx::query("ALTER TABLE reminders ADD COLUMN todo_id INTEGER;")
        .execute(pool)
        .await;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_reminders_todo ON reminders(todo_id);")
        .execute(pool)
        .await?;

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
            "reminders",
            "todos",
            "transactions",
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
            "INSERT INTO todos (room_id, sender_mxid, sender_name, text, priority, completed) \
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
