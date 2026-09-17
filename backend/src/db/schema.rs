use sqlx::SqlitePool;
use tracing::info;

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    info!("Running database table schema initialization...");

    // The ownership column used to be called `sender_mxid`, because it held a
    // Matrix user ID. It now holds an `accounts.id`, so the old name actively
    // lies about what is in it. Renamed first, before anything reads or indexes it.
    for table in ["todos", "reminders", "transactions", "recurring_bills"] {
        let _ = sqlx::query(&format!(
            "ALTER TABLE {table} RENAME COLUMN sender_mxid TO owner_id;"
        ))
        .execute(pool)
        .await;
    }

    crate::db::schema_auth::create_tables(pool).await?;
    crate::db::schema_settings::create_tables(pool).await?;
    crate::db::schema_webhooks::create_tables(pool).await?;
    crate::db::schema_ai::create_tables(pool).await?;
    crate::db::schema_portfolio::create_tables(pool).await?;
    crate::db::schema_footprints::create_tables(pool).await?;
    crate::db::schema_habits::create_tables(pool).await?;
    crate::db::schema_mcp::create_tables(pool).await?;
    crate::db::schema_life::run_life_schema(pool).await?;
    crate::db::schema_bookmarks::create_tables(pool).await?;
    crate::db::schema_rooms::create_tables(pool).await?;
    crate::db::schema_todos::create_tables(pool).await?;
    crate::db::schema_reminders::create_tables(pool).await?;
    crate::db::schema_finance::create_tables(pool).await?;
    crate::db::schema_bills::create_tables(pool).await?;
    crate::db::schema_reactions::create_tables(pool).await?;

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
            "bot_message_targets",
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
            assert!(
                tables.contains(&expected.to_string()),
                "missing table {expected}"
            );
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

        assert_eq!(
            first, third,
            "re-running migrations must not change the schema"
        );
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
        assert!(
            duplicate.is_err(),
            "a duplicate name must be rejected by the index"
        );

        // A different owner may reuse the name.
        sqlx::query("INSERT INTO finance_categories (owner_id, name) VALUES ('@b:x', 'food')")
            .execute(&pool)
            .await
            .unwrap();
    }
}
