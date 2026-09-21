use anyhow::{bail, Context, Result};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
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

impl Database {
    pub async fn connect(database_url: &str, max_connections: u32) -> Result<Self> {
        info!("Connecting to database at {}", database_url);

        // The PostgreSQL rollout is not finished: the schema in `db/schema*.rs`
        // is still SQLite-only DDL and most repositories still reach
        // `sqlite_pool()`. Booting against Postgres would create nothing and
        // then fail per-query, which is a far worse experience than refusing
        // here. See `docs/POSTGRES_MIGRATION.md`; this guard, and the
        // scaffolding accessor below, are deleted together when stage 5 lands.
        if Dialect::from_url(database_url) == Dialect::Postgres {
            bail!(
                "PostgreSQL support is still being rolled out and is not usable yet \
                 (see docs/POSTGRES_MIGRATION.md). Point DATABASE_URL at SQLite, \
                 e.g. sqlite://data/bot_data.db?mode=rwc"
            );
        }

        let pool = connect_pool(database_url, max_connections).await?;

        schema::run_migrations(pool.sqlite_or_err()?)
            .await
            .context("Failed to execute database migrations")?;

        Ok(Self { pool })
    }

    /// The SQLite pool, for the repositories not yet converted to the
    /// dual-dialect macros.
    ///
    /// **Migration scaffolding**, and the reason 406 repository signatures can
    /// stay `&SqlitePool` while modules convert one at a time. It cannot
    /// observe a Postgres pool today because `connect` refuses that URL above;
    /// the two are removed in the same commit.
    pub fn sqlite_pool(&self) -> &SqlitePool {
        self.pool
            .sqlite_or_err()
            .expect("Database::connect refuses Postgres until the rollout completes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_connect_runs_migrations() {
        let db = Database::connect("sqlite::memory:", 1).await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM todos")
            .fetch_one(db.sqlite_pool())
            .await
            .unwrap();
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
        sqlx::query("INSERT INTO web_users (username, password_hash) VALUES ('a', 'h')")
            .execute(db.sqlite_pool())
            .await
            .unwrap();

        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_a_sqlite_url_yields_a_sqlite_pool() {
        let pool = connect_pool("sqlite::memory:", 1).await.unwrap();
        assert_eq!(pool.dialect(), Dialect::Sqlite);
    }

    /// Until the rollout completes, a Postgres URL must be refused at boot with
    /// an explanation, not accepted into a process that would then fail on
    /// every query. Delete this test with the guard it pins.
    #[tokio::test]
    async fn test_postgres_is_refused_at_boot_while_the_rollout_is_unfinished() {
        let err = Database::connect("postgres://localhost/whatever", 1)
            .await
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("still being rolled out"),
            "the refusal must say why, got: {err}"
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
    /// docker run --rm -d -p 55432:5432 -e POSTGRES_PASSWORD=bot --name bot-pg postgres:16
    /// PG_TEST_URL=postgres://postgres:bot@127.0.0.1:55432/postgres \
    ///   cargo test --package matrix-bot connects_to_a_live_postgres -- --ignored
    /// ```
    #[tokio::test]
    #[ignore = "needs a live PostgreSQL server; set PG_TEST_URL"]
    async fn test_connect_pool_connects_to_a_live_postgres() {
        let url = std::env::var("PG_TEST_URL").expect("PG_TEST_URL must be set for this test");
        let pool = connect_pool(&url, 2).await.expect("should connect");
        assert_eq!(pool.dialect(), Dialect::Postgres);

        let one: i64 = crate::db_fetch_scalar!(&pool, i64, "SELECT 1").unwrap();
        assert_eq!(one, 1, "the pool must actually answer a query");
    }
}
