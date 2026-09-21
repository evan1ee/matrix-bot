use anyhow::{Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::str::FromStr;
use tracing::info;

use crate::db::dialect::Dialect;
use crate::db::executor::DbPool;
use crate::db::schema;

#[derive(Clone, Debug)]
pub struct Database {
    pub pool: DbPool,
}

/// Opens a pool for whichever database `database_url` names.
///
/// Split out from `Database::connect` so the connection logic can be tested
/// without also running migrations — the SQLite branch creates directories and
/// sets pragmas, and those are worth asserting on their own.
pub async fn connect_pool(database_url: &str, max_connections: u32) -> Result<DbPool> {
    match Dialect::from_url(database_url) {
        Dialect::Sqlite => {
            // `DATABASE_URL` defaults to `data/bot_data.db`, and that directory
            // is absent on a fresh checkout and in a fresh container.
            if !database_url.contains("mode=memory") {
                let raw_path = database_url
                    .trim_start_matches("sqlite://")
                    .split('?')
                    .next()
                    .unwrap_or("data/bot_data.db");
                let path = std::path::Path::new(raw_path);
                if let Some(parent) = path.parent() {
                    if !parent.as_os_str().is_empty() && !parent.exists() {
                        info!("Creating database directory: {:?}", parent);
                        tokio::fs::create_dir_all(parent)
                            .await
                            .context("Failed to create database directory")?;
                    }
                }
            }

            let options = SqliteConnectOptions::from_str(database_url)?
                .create_if_missing(true)
                .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
                .busy_timeout(std::time::Duration::from_secs(5));

            let pool = SqlitePoolOptions::new()
                .max_connections(max_connections)
                .connect_with(options)
                .await
                .context("Failed to connect to SQLite database")?;

            Ok(DbPool::Sqlite(pool))
        }
        Dialect::Postgres => {
            // No directory to create and no journal mode to set: the server
            // owns its storage. `max_connections` means the same thing on both
            // and stays the caller's decision.
            let pool = PgPoolOptions::new()
                .max_connections(max_connections)
                .connect(database_url)
                .await
                .context("Failed to connect to PostgreSQL database")?;

            Ok(DbPool::Postgres(pool))
        }
    }
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

/// The pool every in-crate test runs against.
///
/// SQLite in memory by default, which keeps `cargo test` instant and
/// dependency-free. Set `TEST_DATABASE_URL` to a PostgreSQL server and the
/// whole suite runs there instead — that is what the second CI job does, and it
/// is the only way a query that happens to be SQLite-flavoured gets caught
/// before production.
///
/// Each call gets its own PostgreSQL schema, because the suite runs in parallel
/// and `search_path` is per connection, not per statement. The schema name
/// carries the process id so concurrent `cargo test` runs cannot collide.
/// Schemas are left behind: the server is a throwaway CI container, and a test
/// that failed is more useful with its rows still in place.
///
/// The pool is returned **unmigrated**; callers run whichever schema they need,
/// exactly as they did when this was an inline `SqlitePool::connect`.
#[cfg(test)]
pub async fn test_pool() -> crate::db::DbPool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let Ok(base) = std::env::var("TEST_DATABASE_URL") else {
        return crate::db::DbPool::Sqlite(
            sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap(),
        );
    };

    let schema = format!(
        "t{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );

    let admin = connect_pool(&base, 1)
        .await
        .expect("TEST_DATABASE_URL must point at a reachable server");
    crate::db_execute!(&admin, &format!("CREATE SCHEMA IF NOT EXISTS {schema}")).unwrap();
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
    crate::db_execute!(&admin, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).unwrap();
    crate::db_execute!(&admin, &format!("CREATE SCHEMA {schema}")).unwrap();

    let separator = if base.contains('?') { "&" } else { "?" };
    let scoped = format!("{base}{separator}options=-c%20search_path%3D{schema}");
    connect_pool(&scoped, 2).await.expect("scoped pool")
}

/// Drops a schema created by `connect_pg_schema`.
#[cfg(test)]
pub async fn drop_pg_schema(schema: &str) {
    let admin = connect_pool(&pg_test_url(), 1).await.expect("admin pool");
    crate::db_execute!(&admin, &format!("DROP SCHEMA IF EXISTS {schema} CASCADE")).unwrap();
}

impl Database {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self> {
        info!("Connecting to database at {}", database_url);

        let pool = connect_pool(database_url, max_connections).await?;

        schema::run_migrations(&pool)
            .await
            .context("Failed to execute database migrations")?;

        Ok(Self { pool })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_connect_runs_migrations() {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();

        let count: i64 = crate::db_scalar_one!(&db.pool, _, "SELECT COUNT(*) FROM todos").unwrap();
        assert_eq!(count, 0, "migrations must have created the table");
    }

    /// `DATABASE_URL` points at `data/bot_data.db` by default, and that
    /// directory does not exist on a fresh checkout or in a fresh container.
    #[tokio::test]
    async fn test_creates_the_database_directory_if_missing() {
        let dir = std::env::temp_dir().join(format!("matrix-bot-pool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let nested = dir.join("nested");
        let url = format!("sqlite://{}/bot.db?mode=rwc", nested.display());

        let db = Database::connect(&url, 1).await.unwrap();
        assert!(nested.exists(), "the parent directory must be created");

        // Usable, not merely present.
        crate::db_execute!(
            &db.pool,
            "INSERT INTO web_users (username, password_hash) VALUES ('a', 'h')"
        )
        .unwrap();

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_a_sqlite_url_yields_a_sqlite_pool() {
        let pool = connect_pool("sqlite::memory:", 1).await.unwrap();
        assert_eq!(pool.dialect(), Dialect::Sqlite);
    }

    /// The refusal that used to live here is gone: every repository, the
    /// schema and the transactions now speak both dialects, so a Postgres URL
    /// is a supported configuration rather than a half-working one. What
    /// remains is that an unreachable server fails as a connection error.
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
    /// connection error rather than being mistaken for SQLite.
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
        assert_eq!(pool.dialect(), Dialect::Postgres);

        // `SELECT 1` is int4 on PostgreSQL and will not decode into i64 — the
        // same mismatch that made the schema use BIGINT everywhere.
        let one: i64 = crate::db_scalar_one!(&pool, i64, "SELECT 1::bigint").unwrap();
        assert_eq!(one, 1, "the pool must actually answer a query");
    }
}
