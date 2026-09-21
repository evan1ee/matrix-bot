use crate::db::DbPool;
use sqlx::SqlitePool;
use tracing::info;

pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::Error> {
    info!("Running database table schema initialization...");

    create_schema(pool).await?;

    // One-shot data conversions, kept out of the DDL because they are a
    // different job: `create_schema` describes the shape of the database, and
    // `backfill` rewrites what is already in it. That distinction is also what
    // lets the PostgreSQL migration be verified independently of the data
    // conversions, which is what the live schema test does.
    crate::db::backfill::run(pool).await?;

    info!("Database migrations applied successfully.");
    Ok(())
}

/// Every `CREATE TABLE` / `CREATE INDEX` / `ALTER TABLE` in the project.
///
/// Separate from `run_migrations` so it can run against either database while
/// `backfill` is still SQLite-only, which is what
/// `test_the_whole_schema_creates_on_a_live_postgres` exercises.
pub async fn create_schema(pool: &DbPool) -> Result<(), sqlx::Error> {
    // The ownership column used to be called `sender_mxid`, because it held a
    // Matrix user ID. It now holds an `accounts.id`, so the old name actively
    // lies about what is in it. Renamed first, before anything reads or indexes it.
    for table in ["todos", "reminders", "transactions", "recurring_bills"] {
        let rename = format!("ALTER TABLE {table} RENAME COLUMN sender_mxid TO owner_id;");
        let _ = crate::db_ddl!(pool, &rename);
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads `sqlite_master`, so it is SQLite-only on purpose. The PostgreSQL
    /// equivalent queries `information_schema` and arrives with the live
    /// migration test in stage 7.
    async fn table_names(pool: &DbPool) -> Vec<String> {
        crate::db_scalar_all!(pool, String, "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap()
    }

    #[tokio::test]
    async fn test_creates_every_table() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
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
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());

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
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();

        crate::db_execute!(
            &pool,
            "INSERT INTO todos (room_id, owner_id, sender_name, text, priority, completed) \
             VALUES ('web', '@a:x', 'a', 'keep me', 'medium', 0)"
        )
        .unwrap();

        run_migrations(&pool).await.unwrap();

        let text: String = crate::db_scalar_one!(&pool, _, "SELECT text FROM todos").unwrap();
        assert_eq!(text, "keep me");
    }

    /// The unique index is what stops a user ending up with two categories of
    /// the same name; the repository relies on it rather than a read-then-write.
    #[tokio::test]
    async fn test_category_name_uniqueness_is_enforced_by_the_schema() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        run_migrations(&pool).await.unwrap();

        crate::db_execute!(
            &pool,
            "INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')"
        )
        .unwrap();

        let duplicate = crate::db_execute!(
            &pool,
            "INSERT INTO finance_categories (owner_id, name) VALUES ('@a:x', 'food')"
        );
        assert!(
            duplicate.is_err(),
            "a duplicate name must be rejected by the index"
        );

        // A different owner may reuse the name.
        crate::db_execute!(
            &pool,
            "INSERT INTO finance_categories (owner_id, name) VALUES ('@b:x', 'food')"
        )
        .unwrap();
    }

    /// The stage 4 acceptance test: every `CREATE TABLE` in this project, run
    /// against a real PostgreSQL. Nothing short of this shows that the DDL
    /// tokens, the portable spellings and the plpgsql trigger functions are
    /// right — SQLite accepts plenty that Postgres rejects, and vice versa.
    ///
    /// `#[ignore]`d because it needs a live server:
    ///
    /// ```text
    /// docker run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=bot --name bot-pg postgres:18
    /// PG_TEST_URL=postgres://postgres:bot@127.0.0.1:55432/postgres     ///   cargo test --package matrix-bot live_postgres -- --ignored --test-threads=1
    /// ```
    ///
    /// It works in a throwaway schema so it neither depends on nor leaves
    /// behind state, and so it can be run repeatedly.
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_the_whole_schema_creates_on_a_live_postgres() {
        let pool = crate::db::pool::connect_pg_schema("migration_check").await;

        create_schema(&pool)
            .await
            .expect("the whole schema must create on PostgreSQL");

        // Re-running must be a no-op, which is the promise every
        // `IF NOT EXISTS` in the schema makes. Postgres has no
        // CREATE TRIGGER IF NOT EXISTS, so this is really a test of the
        // DROP-then-CREATE in `ddl::guard_trigger`.
        create_schema(&pool)
            .await
            .expect("the schema must be idempotent on PostgreSQL");

        let tables: Vec<String> = crate::db_fetch_all!(
            &pool,
            (String,),
            "SELECT table_name FROM information_schema.tables              WHERE table_schema = 'migration_check' ORDER BY table_name"
        )
        .unwrap()
        .into_iter()
        .map(|r| r.0)
        .collect();

        for expected in [
            "accounts",
            "todos",
            "reminders",
            "transactions",
            "recurring_bills",
            "portfolio_holdings",
            "portfolio_audit_log",
            "user_settings",
        ] {
            assert!(
                tables.iter().any(|t| t == expected),
                "PostgreSQL schema is missing `{expected}`; got {tables:?}"
            );
        }

        crate::db::pool::drop_pg_schema("migration_check").await;
    }

    /// The audit log must be append-only on PostgreSQL too, not merely have
    /// triggers created. `accounts/merge.rs` depends on the UPDATE guard
    /// firing, so a trigger that exists but does not abort would be worse than
    /// none at all.
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_the_audit_log_is_append_only_on_a_live_postgres() {
        let pool = crate::db::pool::connect_pg_schema("trigger_check").await;
        create_schema(&pool).await.unwrap();

        crate::db_execute!(
            &pool,
            "INSERT INTO portfolio_audit_log (owner_id, action, amount_minor, currency)              VALUES (?, ?, ?, ?)",
            "acct_a",
            "buy",
            1_000_i64,
            "USD"
        )
        .expect("an audit row must be insertable");

        assert!(
            crate::db_execute!(&pool, "UPDATE portfolio_audit_log SET owner_id = 'acct_b'")
                .is_err(),
            "PostgreSQL must refuse to update an audit row"
        );
        assert!(
            crate::db_execute!(&pool, "DELETE FROM portfolio_audit_log").is_err(),
            "PostgreSQL must refuse to delete an audit row"
        );

        crate::db::pool::drop_pg_schema("trigger_check").await;
    }

    /// The stage 5 acceptance test: a real repository round-trip on PostgreSQL.
    ///
    /// Compiling proves the 553 converted call sites type-check; it proves
    /// nothing about whether the SQL they now send is accepted. This exercises
    /// the ordinary todo path — insert with `RETURNING id`, read back, filter,
    /// update, count — against a live server, which is where placeholder
    /// renumbering, `BIGINT` widening and `RETURNING` either work or do not.
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_a_repository_round_trips_on_a_live_postgres() {
        use crate::modules::todo::models::TodoPriority;
        use crate::modules::todo::repository::TodoRepository;

        let pool = crate::db::pool::connect_pg_schema("repo_check").await;
        create_schema(&pool).await.unwrap();

        let todo = TodoRepository::add(
            &pool,
            "!room:example.org",
            "acct_roundtrip",
            "Evan",
            "buy milk",
            TodoPriority::High,
            None,
            None,
        )
        .await
        .expect("insert must work on PostgreSQL");
        assert!(todo.id > 0, "RETURNING id must yield a real id");
        assert_eq!(todo.text, "buy milk");

        let listed = TodoRepository::list_all_user(&pool, "acct_roundtrip", None, 10, 0)
            .await
            .expect("list must work on PostgreSQL");
        assert_eq!(listed.len(), 1, "the row must read back");
        assert_eq!(listed[0].id, todo.id);

        let updated = TodoRepository::set_completed(&pool, todo.id, true)
            .await
            .expect("update must work on PostgreSQL");
        assert!(updated.is_some(), "rows_affected must be reported");

        crate::db::pool::drop_pg_schema("repo_check").await;
    }
}
