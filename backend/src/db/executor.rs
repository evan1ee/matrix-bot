//! The pool and transaction types, and the `?`-style query macros still used by
//! code not yet converted to compile-checked `query!`. The macros are removed
//! once every repository has moved; new code uses `sqlx::query!` directly.

/// The application's PostgreSQL pool.
pub type DbPool = sqlx::PgPool;

/// A PostgreSQL transaction.
pub type DbTx<'c> = sqlx::Transaction<'c, sqlx::Postgres>;

#[doc(hidden)]
pub use crate::db::dialect::render_placeholders as __render;

/// `query_as` + `fetch_all` into `Vec<$ty>`.
///
/// Pass `_` for `$ty` where the row type is inferred from context, which
/// expands to `query_as::<_, _>` exactly as hand-written sqlx would.
///
/// ```ignore
/// let rows = crate::db_fetch_all!(pool, Todo, "SELECT * FROM todos WHERE owner_id = ?", owner)?;
/// ```
#[macro_export]
macro_rules! db_fetch_all {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(__p).await
    }};
}

/// `query_as` + `fetch_one` into `$ty`.
#[macro_export]
macro_rules! db_fetch_one {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(__p).await
    }};
}

/// `query_as` + `fetch_optional` into `Option<$ty>`.
#[macro_export]
macro_rules! db_fetch_optional {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_optional(__p).await
    }};
}

/// `query_scalar` + `fetch_all` into `Vec<$ty>`.
#[macro_export]
macro_rules! db_scalar_all {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(__p).await
    }};
}

/// `query_scalar` + `fetch_one` into `$ty`, e.g. a `COUNT(*)`.
#[macro_export]
macro_rules! db_scalar_one {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(__p).await
    }};
}

/// `query_scalar` + `fetch_optional` into `Option<$ty>`.
#[macro_export]
macro_rules! db_scalar_optional {
    ($pool:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_optional(__p).await
    }};
}

/// A statement run for its effect, yielding `rows_affected()` directly.
///
/// It returns the count rather than the driver's `QueryResult` because that is
/// the only part this project ever reads. `last_insert_rowid()` is SQLite-only
/// and has no equivalent here: an insert that needs its new id uses
/// `RETURNING id` with `db_scalar_one!`.
#[macro_export]
macro_rules! db_execute {
    ($pool:expr, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query(&__sql)$(.bind($bind))*.execute(__p).await.map(|r| r.rows_affected())
    }};
}

/// `query_as` + `fetch_all` where the number of binds is decided at runtime.
///
/// A handful of list endpoints build their `WHERE` clause from optional filters,
/// so the bind count varies with the request. `$dyn` is an iterable of
/// same-typed values bound first, in order, followed by any fixed `$bind`
/// arguments — which is exactly the shape those call sites already had
/// (filter patterns, then `LIMIT`/`OFFSET`).
#[macro_export]
macro_rules! db_fetch_all_dyn {
    ($pool:expr, $ty:ty, $sql:expr, $dyn:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        let mut __q = sqlx::query_as::<_, $ty>(&__sql);
        for __v in $dyn {
            __q = __q.bind(__v);
        }
        __q$(.bind($bind))*.fetch_all(__p).await
    }};
}

/// `query_as` + `fetch_one` with a runtime-decided bind count. See
/// `db_fetch_all_dyn!`.
#[macro_export]
macro_rules! db_fetch_one_dyn {
    ($pool:expr, $ty:ty, $sql:expr, $dyn:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __p: &$crate::db::DbPool = $pool;
        let __sql = $crate::db::executor::__render(__sql_src);
        let mut __q = sqlx::query_as::<_, $ty>(&__sql);
        for __v in $dyn {
            __q = __q.bind(__v);
        }
        __q$(.bind($bind))*.fetch_one(__p).await
    }};
}

/// A statement inside a transaction, yielding `rows_affected()`.
#[macro_export]
macro_rules! tx_execute {
    ($tx:expr, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query(&__sql)$(.bind($bind))*.execute(&mut **__t).await.map(|r| r.rows_affected())
    }};
}

/// `query_as` + `fetch_all` inside a transaction.
#[macro_export]
macro_rules! tx_fetch_all {
    ($tx:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(&mut **__t).await
    }};
}

/// `query_as` + `fetch_one` inside a transaction.
#[macro_export]
macro_rules! tx_fetch_one {
    ($tx:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(&mut **__t).await
    }};
}

/// `query_as` + `fetch_optional` inside a transaction.
#[macro_export]
macro_rules! tx_fetch_optional {
    ($tx:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_as::<_, $ty>(&__sql)$(.bind($bind))*.fetch_optional(&mut **__t).await
    }};
}

/// `query_scalar` + `fetch_one` inside a transaction.
#[macro_export]
macro_rules! tx_scalar_one {
    ($tx:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_one(&mut **__t).await
    }};
}

/// `query_scalar` + `fetch_all` inside a transaction.
#[macro_export]
macro_rules! tx_scalar_all {
    ($tx:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_all(&mut **__t).await
    }};
}

/// `query_scalar` + `fetch_optional` inside a transaction.
#[macro_export]
macro_rules! tx_scalar_optional {
    ($tx:expr, $ty:ty, $sql:expr $(, $bind:expr)* $(,)?) => {{
        let __sql_src = $sql;
        let __t: &mut $crate::db::DbTx<'_> = $tx;
        let __sql = $crate::db::executor::__render(__sql_src);
        sqlx::query_scalar::<_, $ty>(&__sql)$(.bind($bind))*.fetch_optional(&mut **__t).await
    }};
}

#[cfg(test)]
mod tests {
    use sqlx::FromRow;

    #[derive(Debug, FromRow, PartialEq)]
    struct Row {
        id: i64,
        name: String,
        active: bool,
    }

    async fn pool() -> crate::db::DbPool {
        let pool = crate::db::pool::test_pool().await;
        sqlx::query(
            "CREATE TABLE t (id BIGINT PRIMARY KEY, name TEXT NOT NULL, active BOOLEAN NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn test_execute_insert_and_fetch_all_round_trip() {
        let pool = pool().await;

        let affected = crate::db_execute!(
            &pool,
            "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
            1_i64,
            "alpha",
            true
        )
        .unwrap();
        assert_eq!(affected, 1);

        let rows = crate::db_fetch_all!(&pool, Row, "SELECT id, name, active FROM t").unwrap();
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
        crate::db_execute!(
            &pool,
            "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
            7_i64,
            "seven",
            false
        )
        .unwrap();

        let hit = crate::db_fetch_optional!(
            &pool,
            Row,
            "SELECT id, name, active FROM t WHERE name = ? AND active = ?",
            "seven",
            false
        )
        .unwrap();
        assert_eq!(hit.map(|r| r.id), Some(7));

        let miss = crate::db_fetch_optional!(
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
            crate::db_execute!(
                &pool,
                "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
                id,
                "x",
                true
            )
            .unwrap();
        }
        let count =
            crate::db_scalar_one!(&pool, i64, "SELECT COUNT(*) FROM t WHERE active = ?", true)
                .unwrap();
        assert_eq!(count, 3);
    }

    /// `RETURNING` is how an insert learns its own id.
    /// `last_insert_rowid()` is SQLite-only and must not be reintroduced.
    #[tokio::test]
    async fn test_returning_yields_the_new_id_without_last_insert_rowid() {
        let pool = pool().await;
        let id = crate::db_scalar_one!(
            &pool,
            i64,
            "INSERT INTO t (id, name, active) VALUES (?, ?, ?) RETURNING id",
            99_i64,
            "generated",
            true
        )
        .unwrap();
        assert!(id > 0);
    }

    /// 81 call sites in this project call `query_as`/`query_scalar` with no
    /// turbofish and let the type fall out of context. They convert by passing
    /// `_` as `$ty`, which expands to `query_as::<_, _>` — this pins that it
    /// compiles and infers, rather than each of those sites discovering it.
    #[tokio::test]
    async fn test_an_underscore_type_infers_from_the_binding() {
        let pool = pool().await;
        crate::db_execute!(
            &pool,
            "INSERT INTO t (id, name, active) VALUES (?, ?, ?)",
            5_i64,
            "five",
            true
        )
        .unwrap();

        let rows: Vec<Row> =
            crate::db_fetch_all!(&pool, _, "SELECT id, name, active FROM t").unwrap();
        assert_eq!(rows.len(), 1);

        let name: String =
            crate::db_scalar_one!(&pool, _, "SELECT name FROM t WHERE id = ?", 5_i64).unwrap();
        assert_eq!(name, "five");

        let missing: Option<String> =
            crate::db_scalar_optional!(&pool, _, "SELECT name FROM t WHERE id = ?", 99_i64)
                .unwrap();
        assert!(missing.is_none());

        let all: Vec<i64> = crate::db_scalar_all!(&pool, _, "SELECT id FROM t").unwrap();
        assert_eq!(all, vec![5]);
    }
}
