use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use tracing::info;

pub type DbPool = sqlx::PgPool;
pub type DbTx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

#[derive(Clone, Debug)]
pub struct Database {
    pub pool: DbPool,
}

/// Opens the PostgreSQL pool. Split from `Database::connect` so tests can get a
/// pool without running migrations.
pub async fn connect_pool(database_url: &str, max_connections: u32) -> Result<DbPool> {
    crate::config::validate_database_url(database_url)?;
    PgPoolOptions::new()
        .max_connections(max_connections)
        .acquire_timeout(std::time::Duration::from_secs(10))
        .connect(database_url)
        .await
        .context("Failed to connect to PostgreSQL")
}

#[cfg(test)]
pub fn lazy_pool(database_url: &str) -> DbPool {
    PgPoolOptions::new()
        .connect_lazy(database_url)
        .expect("lazy pool")
}

/// The PostgreSQL URL the `#[ignore]`d live tests connect to.
///
/// Lives here because `config.rs`'s "only config reads the environment" guard
/// allow-lists this file, and duplicating the read into other modules would
/// mean growing that list further.
#[cfg(test)]
pub fn pg_test_url() -> String {
    std::env::var("PG_TEST_URL").expect("PG_TEST_URL must be set for the live PostgreSQL tests")
}

/// The PostgreSQL server every in-crate test runs against.
#[cfg(test)]
pub fn test_database_url() -> String {
    std::env::var("TEST_DATABASE_URL").expect(
        "TEST_DATABASE_URL must point at a PostgreSQL server, e.g. \
         postgres://postgres:bot@localhost:5432/postgres (see AGENTS.md)",
    )
}

/// The pool every in-crate test runs against.
///
/// Each call gets its own PostgreSQL schema, because the suite runs in parallel
/// and `search_path` is per connection, not per statement. The schema name
/// carries the process id so concurrent `cargo test` runs cannot collide, and
/// is dropped first in case a dead process with the same pid left it behind.
///
/// Schemas outlive the test so a failure keeps its rows, but not the process:
/// the next test process to start reaps them (see [`crate::db::test_schemas`]).
///
/// The pool is returned **unmigrated**; callers run whichever schema they need.
#[cfg(test)]
pub async fn test_pool() -> crate::db::DbPool {
    use crate::db::test_schemas;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let base = test_database_url();
    test_schemas::hold_liveness_lock(&base);
    let schema =
        test_schemas::schema_name(std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));

    let admin = connect_pool(&base, 1)
        .await
        .expect("TEST_DATABASE_URL must point at a reachable server");
    test_schemas::drop_schema(&admin, &schema).await.unwrap();
    sqlx::QueryBuilder::<sqlx::Postgres>::new("CREATE SCHEMA ")
        .push(&schema)
        .build()
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;

    let separator = if base.contains('?') { "&" } else { "?" };
    let scoped = format!("{base}{separator}options=-c%20search_path%3D{schema}");
    connect_pool(&scoped, 2).await.expect("scoped test pool")
}

/// A pool pointed at a throwaway PostgreSQL schema, for the live tests.
///
/// The search path has to travel in the connection options, not in a `SET`
/// statement: a `SET` applies to one session, and a pool hands successive
/// queries to different connections. That is not hypothetical — it is how the
/// first run of these tests failed, creating `matrix_links` on one connection
/// and then looking for it from another.
#[cfg(test)]
pub async fn connect_pg_schema(schema: &str) -> crate::db::DbPool {
    let base = pg_test_url();
    let admin = connect_pool(&base, 1).await.expect("admin pool");
    sqlx::QueryBuilder::<sqlx::Postgres>::new("DROP SCHEMA IF EXISTS ")
        .push(schema)
        .push(" CASCADE")
        .build()
        .execute(&admin)
        .await
        .unwrap();
    sqlx::QueryBuilder::<sqlx::Postgres>::new("CREATE SCHEMA ")
        .push(schema)
        .build()
        .execute(&admin)
        .await
        .unwrap();

    let separator = if base.contains('?') { "&" } else { "?" };
    let scoped = format!("{base}{separator}options=-c%20search_path%3D{schema}");
    connect_pool(&scoped, 2).await.expect("scoped pool")
}

/// Drops a schema created by `connect_pg_schema`.
#[cfg(test)]
pub async fn drop_pg_schema(schema: &str) {
    let admin = connect_pool(&pg_test_url(), 1).await.expect("admin pool");
    sqlx::QueryBuilder::<sqlx::Postgres>::new("DROP SCHEMA IF EXISTS ")
        .push(schema)
        .push(" CASCADE")
        .build()
        .execute(&admin)
        .await
        .unwrap();
}

impl Database {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self> {
        info!("Connecting to PostgreSQL");

        let pool = connect_pool(database_url, max_connections).await?;

        crate::db::migrate::run_migrations(&pool)
            .await
            .context("Failed to apply database migrations")?;

        Ok(Self { pool })
    }
}

/// Inserts the `accounts` row a test fixture's owner id refers to.
///
/// Every `owner_id` / `account_id` column is a foreign key to `accounts(id)`,
/// so a fixture that writes as `AccountId::from("alice")` must create that
/// account first. Idempotent, so a test can call it for each id it uses.
#[cfg(test)]
pub async fn seed_account(pool: &crate::db::DbPool, id: &crate::domain::accounts::AccountId) {
    sqlx::query!(
        "INSERT INTO accounts (id, display_name, status) VALUES ($1, 'test', 'active') \
         ON CONFLICT DO NOTHING",
        id.as_uuid()
    )
    .execute(pool)
    .await
    .unwrap();
}

/// [`seed_account`] for several fixture owners, named the way the test names
/// them (`"@alice:a"`, `"acct_alice"`).
#[cfg(test)]
pub async fn seed_accounts(pool: &crate::db::DbPool, names: &[&str]) {
    for name in names {
        seed_account(pool, &crate::domain::accounts::AccountId::from(*name)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connect_pool_refuses_sqlite() {
        let err = connect_pool("sqlite::memory:", 1).await.unwrap_err();
        assert!(err.to_string().contains("DATABASE_URL"));
    }

    /// The refusal that used to live here is gone: every repository, the
    /// schema and the transactions now speak PostgreSQL only, so a Postgres URL
    /// is the only supported configuration. What remains is that an
    /// unreachable server fails as a connection error.
    #[tokio::test]
    async fn test_a_postgres_url_is_no_longer_refused_before_connecting() {
        let err = Database::connect("postgres://127.0.0.1:1/nonexistent", 1)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("PostgreSQL"),
            "must fail on the connection, not on a policy refusal, got: {err}"
        );
    }

    /// The connection layer itself is finished, so this is about the *driver*,
    /// not the rollout: a Postgres URL pointing nowhere must fail as a
    /// connection error.
    #[tokio::test]
    async fn test_a_postgres_url_is_routed_to_the_postgres_driver() {
        let err = connect_pool("postgres://127.0.0.1:1/nonexistent", 1)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("PostgreSQL"),
            "must be attempted on the postgres driver, got: {err}"
        );
    }

    /// Proves `connect_pool` really opens a PostgreSQL pool, which no amount of
    /// unit testing against a refused connection can show. `#[ignore]`d because
    /// it needs a live server, matching how the other live checks in this crate
    /// are gated:
    ///
    /// ```text
    /// docker run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=bot --name bot-pg postgres:18
    /// PG_TEST_URL=postgres://postgres:bot@127.0.0.1:55432/postgres \
    ///   cargo test --package matrix-bot connects_to_a_live_postgres -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_connect_pool_connects_to_a_live_postgres() {
        let url = pg_test_url();
        let pool = connect_pool(&url, 2).await.expect("should connect");

        // `SELECT 1` is int4 on PostgreSQL and will not decode into i64 — the
        // same mismatch that made the schema use BIGINT everywhere.
        let one: i64 = sqlx::query_scalar!(r#"SELECT 1::bigint AS "one!""#)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(one, 1, "the pool must actually answer a query");
    }

    /// The whole boot path on PostgreSQL: `Database::connect`, which now runs
    /// the migrator, then an ordinary write and read.
    ///
    /// This is the claim "the bot runs on PostgreSQL" reduced to a test. Every
    /// other live test exercises one layer; this one starts from a connection
    /// string, as the process does.
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_the_bot_boots_against_a_live_postgres() {
        // A dedicated schema so this neither sees nor leaves shared state.
        let scoped = {
            let base = pg_test_url();
            let admin = connect_pool(&base, 1).await.unwrap();
            sqlx::query("DROP SCHEMA IF EXISTS boot_check CASCADE")
                .execute(&admin)
                .await
                .unwrap();
            sqlx::query("CREATE SCHEMA boot_check")
                .execute(&admin)
                .await
                .unwrap();
            let sep = if base.contains('?') { "&" } else { "?" };
            format!("{base}{sep}options=-c%20search_path%3Dboot_check")
        };

        let db = Database::connect(&scoped, 2)
            .await
            .expect("Database::connect must run every migration on PostgreSQL");

        let boot_id = uuid::Uuid::new_v4();
        sqlx::query!(
            "INSERT INTO accounts (id, display_name, status) VALUES ($1, $2, $3)",
            boot_id,
            "Evan",
            "unclaimed"
        )
        .execute(&db.pool)
        .await
        .expect("an ordinary write must work after boot");

        let status: String =
            sqlx::query_scalar!("SELECT status FROM accounts WHERE id = $1", boot_id)
                .fetch_one(&db.pool)
                .await
                .expect("an ordinary read must work after boot");
        assert_eq!(status, "unclaimed");

        crate::db::pool::drop_pg_schema("boot_check").await;
    }
}
