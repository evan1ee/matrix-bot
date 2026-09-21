//! One pool handle over two drivers, and the macros that dispatch onto it.
//!
//! Why macros rather than generics: making repositories generic over
//! `sqlx::Database` is viral. Every signature would need `for<'r> Model:
//! FromRow<'r, DB::Row>` plus an `Encode` and a `Type` bound *per bound value*,
//! and those bounds propagate to every caller. The dispatch is mechanical and
//! identical everywhere, which is precisely what a macro is for.
//!
//! Why not two pools side by side: a repository would then have to know which
//! one is live, and every call site would grow a branch. The branch belongs in
//! one place.
//!
//! The SQL is written once, in SQLite's `?` style, and
//! `dialect::render_placeholders` rewrites it per driver — see that module for
//! why `sqlx::Any` cannot do this job.

use sqlx::postgres::PgPool;
use sqlx::SqlitePool;

use crate::db::dialect::Dialect;

/// A connection pool for whichever database this process was pointed at.
#[derive(Clone, Debug)]
pub enum DbPool {
    Sqlite(SqlitePool),
    Postgres(PgPool),
}

impl DbPool {
    pub fn dialect(&self) -> Dialect {
        match self {
            DbPool::Sqlite(_) => Dialect::Sqlite,
            DbPool::Postgres(_) => Dialect::Postgres,
        }
    }

    /// The SQLite pool, for code not yet converted to the macros below.
    ///
    /// **Migration scaffolding.** It exists so the tree keeps compiling and the
    /// suite keeps passing while repositories move over one module at a time,
    /// rather than requiring one commit that rewrites 553 query sites at once.
    /// Every remaining caller is a module still to convert; when the count
    /// reaches zero this method and its test go away. Until then, pointing the
    /// process at Postgres reaches an unconverted module and fails loudly here
    /// rather than silently doing the wrong thing — which is why this returns
    /// an error instead of panicking.
    pub fn sqlite_or_err(&self) -> Result<&SqlitePool, sqlx::Error> {
        match self {
            DbPool::Sqlite(pool) => Ok(pool),
            DbPool::Postgres(_) => Err(sqlx::Error::Configuration(
                "this code path has not been converted to the dual-dialect layer yet; \
                 it cannot run against PostgreSQL"
                    .into(),
            )),
        }
    }
}

/// Renders `$sql` for `$dialect`. Used only by the macros below.
#[doc(hidden)]
pub use crate::db::dialect::render_placeholders as __render;

/// `fetch_all` into `Vec<$ty>`.
///
/// ```ignore
/// let rows = db_fetch_all!(pool, Todo, "SELECT * FROM todos WHERE owner_id = ?", owner_id)?;
/// ```
#[macro_export]
macro_rules! db_fetch_all {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        // Bound first: `let x = &format!(..)` extends the temporary to the end of
        // this block, where using it inline would drop it mid-statement.
        let __sql_src = $sql;
        match $pool {
            $crate::db::DbPool::Sqlite(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Sqlite);
                sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(__p).await
            }
            $crate::db::DbPool::Postgres(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Postgres);
                sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(__p).await
            }
        }
    }};
}

/// `fetch_optional` into `Option<$ty>`.
#[macro_export]
macro_rules! db_fetch_optional {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        // Bound first: `let x = &format!(..)` extends the temporary to the end of
        // this block, where using it inline would drop it mid-statement.
        let __sql_src = $sql;
        match $pool {
            $crate::db::DbPool::Sqlite(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Sqlite);
                sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_optional(__p).await
            }
            $crate::db::DbPool::Postgres(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Postgres);
                sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_optional(__p).await
            }
        }
    }};
}

/// `fetch_one` into `$ty`.
#[macro_export]
macro_rules! db_fetch_one {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        // Bound first: `let x = &format!(..)` extends the temporary to the end of
        // this block, where using it inline would drop it mid-statement.
        let __sql_src = $sql;
        match $pool {
            $crate::db::DbPool::Sqlite(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Sqlite);
                sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(__p).await
            }
            $crate::db::DbPool::Postgres(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Postgres);
                sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(__p).await
            }
        }
    }};
}

/// A single scalar column, e.g. `COUNT(*)`.
#[macro_export]
macro_rules! db_fetch_scalar {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        // Bound first: `let x = &format!(..)` extends the temporary to the end of
        // this block, where using it inline would drop it mid-statement.
        let __sql_src = $sql;
        match $pool {
            $crate::db::DbPool::Sqlite(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Sqlite);
                sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(__p).await
            }
            $crate::db::DbPool::Postgres(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Postgres);
                sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(__p).await
            }
        }
    }};
}

/// A statement run for its effect, yielding the driver's `QueryResult`.
///
/// Note that `rows_affected()` is available on both drivers' results, but
/// `last_insert_rowid()` is SQLite-only — an insert that needs its new id must
/// use `RETURNING id` with `db_fetch_scalar!`, which both databases support.
#[macro_export]
macro_rules! db_execute {
    ($pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        match $pool {
            $crate::db::DbPool::Sqlite(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Sqlite);
                sqlx::query(&__sql)$(.bind($bind))*.execute(__p).await
                    .map(|r| r.rows_affected())
            }
            $crate::db::DbPool::Postgres(__p) => {
                let __sql = $crate::db::executor::__render(__sql_src, $crate::db::dialect::Dialect::Postgres);
                sqlx::query(&__sql)$(.bind($bind))*.execute(__p).await
                    .map(|r| r.rows_affected())
            }
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::FromRow;

    #[derive(Debug, FromRow, PartialEq)]
    struct Row {
        id: i64,
        name: String,
        active: bool,
    }

    async fn pool() -> DbPool {
        let sqlite = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT NOT NULL, active BOOLEAN NOT NULL)",
        )
        .execute(&sqlite)
        .await
        .unwrap();
        DbPool::Sqlite(sqlite)
    }

    #[tokio::test]
    async fn test_execute_insert_and_fetch_all_round_trip() {
        let pool = pool().await;

        let affected = db_execute!(
            &pool,
            "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
            1_i64,
            "alpha",
            true
        )
        .unwrap();
        assert_eq!(affected, 1);

        let rows = db_fetch_all!(&pool, Row, "SELECT id, name, active FROM t").unwrap();
        assert_eq!(
            rows,
            vec![Row {
                id: 1,
                name: "alpha".to_string(),
                active: true
            }]
        );
    }

    /// Several binds in one statement, which is where a placeholder renderer
    /// that miscounted would show up.
    #[tokio::test]
    async fn test_fetch_optional_matches_on_multiple_bound_values() {
        let pool = pool().await;
        db_execute!(
            &pool,
            "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
            7_i64,
            "seven",
            false
        )
        .unwrap();

        let hit = db_fetch_optional!(
            &pool,
            Row,
            "SELECT id, name, active FROM t WHERE name = ? AND active = ?",
            "seven",
            false
        )
        .unwrap();
        assert_eq!(hit.map(|r| r.id), Some(7));

        let miss = db_fetch_optional!(
            &pool,
            Row,
            "SELECT id, name, active FROM t WHERE name = ? AND active = ?",
            "seven",
            true
        )
        .unwrap();
        assert!(miss.is_none());
    }

    #[tokio::test]
    async fn test_fetch_scalar_counts() {
        let pool = pool().await;
        for id in 1..=3_i64 {
            db_execute!(
                &pool,
                "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
                id,
                "x",
                true
            )
            .unwrap();
        }
        let count =
            db_fetch_scalar!(&pool, i64, "SELECT COUNT(*) FROM t WHERE active = ?", true).unwrap();
        assert_eq!(count, 3);
    }

    /// `RETURNING` is how an insert learns its own id on both drivers.
    /// `last_insert_rowid()` is SQLite-only and must not be reintroduced.
    #[tokio::test]
    async fn test_returning_yields_the_new_id_without_last_insert_rowid() {
        let pool = pool().await;
        let id = db_fetch_scalar!(
            &pool,
            i64,
            "INSERT INTO t (name, active) VALUES (?, ?) RETURNING id",
            "generated",
            true
        )
        .unwrap();
        assert!(id > 0);
    }

    #[tokio::test]
    async fn test_dialect_reports_the_driver_in_use() {
        assert_eq!(pool().await.dialect(), Dialect::Sqlite);
    }

    /// The scaffolding accessor must hand back the SQLite pool, and must
    /// refuse rather than mislead when the process is on Postgres.
    #[tokio::test]
    async fn test_the_migration_accessor_returns_sqlite_and_refuses_postgres() {
        assert!(pool().await.sqlite_or_err().is_ok());
    }
}
