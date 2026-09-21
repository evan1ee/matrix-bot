//! The handful of DDL constructs that have no form both databases accept.
//!
//! Most of this project's schema is already portable and stays written once:
//! `TEXT`, `INTEGER`, `BOOLEAN`, `TIMESTAMP DEFAULT CURRENT_TIMESTAMP`,
//! `CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS` and `UNIQUE`
//! all mean the same thing on SQLite and PostgreSQL. The best-effort
//! `ALTER TABLE ... ADD COLUMN` statements need nothing either: their errors
//! are already discarded with `let _ =`, and a duplicate-column error on
//! Postgres is discarded exactly as SQLite's is.
//!
//! What is left is substituted through **named tokens** rather than by
//! pattern-matching the SQL. That is the whole point: a regex translator fails
//! silently when it meets a spelling it does not know, whereas an unsubstituted
//! `{{TOKEN}}` is not valid SQL in either dialect and fails loudly at
//! migration time, which is boot. Adding a construct means adding a token here,
//! not hoping a rule generalises.

use std::borrow::Cow;

use crate::db::dialect::Dialect;

/// An auto-assigning integer primary key.
///
/// SQLite's `INTEGER PRIMARY KEY AUTOINCREMENT` and PostgreSQL's
/// `BIGSERIAL PRIMARY KEY` are the same intent with no shared spelling. Both
/// read back into the `i64` every model uses for `id`.
pub const PK: &str = "{{PK}}";

/// The current timestamp as a *default expression*.
///
/// Bare `CURRENT_TIMESTAMP` is portable and is used directly throughout the
/// schema. This token exists for the columns declared `TEXT` that defaulted to
/// SQLite's `datetime('now')`: Postgres will not put a timestamp in a text
/// column without an explicit cast.
pub const NOW_TEXT: &str = "{{NOW_TEXT}}";

/// Expands the tokens above for `dialect`.
///
/// SQL containing no token is returned borrowed, so the common statement
/// allocates nothing.
pub fn render(sql: &str, dialect: Dialect) -> Cow<'_, str> {
    if !sql.contains("{{") {
        return Cow::Borrowed(sql);
    }

    let (pk, now_text) = match dialect {
        Dialect::Sqlite => ("INTEGER PRIMARY KEY AUTOINCREMENT", "(datetime('now'))"),
        Dialect::Postgres => ("BIGSERIAL PRIMARY KEY", "(NOW()::text)"),
    };

    Cow::Owned(sql.replace(PK, pk).replace(NOW_TEXT, now_text))
}

/// The name of the trigger guarding the audit log against UPDATE.
const AUDIT_UPDATE: &str = "prevent_portfolio_audit_update";
/// The name of the trigger guarding the audit log against DELETE.
const AUDIT_DELETE: &str = "prevent_portfolio_audit_delete";

/// Statements creating one audit-log guard trigger, idempotently.
///
/// SQLite expresses this as a trigger whose body calls `RAISE(ABORT, ...)`.
/// PostgreSQL has neither `CREATE TRIGGER IF NOT EXISTS` nor statement triggers
/// of that shape: it needs a `plpgsql` function plus a row trigger, and
/// re-running the migration has to `DROP TRIGGER IF EXISTS` first. They are
/// different enough that translating them textually would be pretending, so
/// each is written out and a *list* is returned — Postgres needs three
/// statements where SQLite needs one.
///
/// **Load-bearing for `accounts/merge.rs`.** That module relies on the UPDATE
/// trigger actually firing: `UPDATE OR IGNORE` does *not* suppress a trigger
/// abort, which `docs/REFACTOR_PROGRESS.md` records as measured behaviour.
/// Whatever replaces `UPDATE OR IGNORE` in stage 5 has to preserve that, so
/// these triggers must genuinely exist on both databases.
fn guard_trigger(dialect: Dialect, name: &str, event: &str, message: &str) -> Vec<String> {
    match dialect {
        Dialect::Sqlite => vec![format!(
            "CREATE TRIGGER IF NOT EXISTS {name} BEFORE {event} ON portfolio_audit_log              BEGIN SELECT RAISE(ABORT, '{message}'); END;"
        )],
        Dialect::Postgres => vec![
            format!(
                "CREATE OR REPLACE FUNCTION {name}() RETURNS trigger AS $fn$                  BEGIN RAISE EXCEPTION '{message}'; END; $fn$ LANGUAGE plpgsql;"
            ),
            drop_trigger(Dialect::Postgres, name),
            format!(
                "CREATE TRIGGER {name} BEFORE {event} ON portfolio_audit_log                  FOR EACH ROW EXECUTE FUNCTION {name}();"
            ),
        ],
    }
}

/// Drops one audit-log guard trigger if present.
///
/// SQLite names a trigger globally; PostgreSQL scopes it to its table and needs
/// the `ON` clause. `modules::portfolio::audit::absorb_ledger` drops the UPDATE
/// guard to move a ledger between owners, so this cannot live only in the
/// schema module.
pub fn drop_trigger(dialect: Dialect, name: &str) -> String {
    match dialect {
        Dialect::Sqlite => format!("DROP TRIGGER IF EXISTS {name};"),
        Dialect::Postgres => format!("DROP TRIGGER IF EXISTS {name} ON portfolio_audit_log;"),
    }
}

/// Creates the UPDATE guard. Also used to *restore* it after `absorb_ledger`.
pub fn audit_update_trigger(dialect: Dialect) -> Vec<String> {
    guard_trigger(
        dialect,
        AUDIT_UPDATE,
        "UPDATE",
        "Audit log entries are immutable and cannot be updated",
    )
}

/// Creates the DELETE guard.
pub fn audit_delete_trigger(dialect: Dialect) -> Vec<String> {
    guard_trigger(
        dialect,
        AUDIT_DELETE,
        "DELETE",
        "Audit log entries cannot be deleted",
    )
}

/// Drops the UPDATE guard, by name, for `absorb_ledger`.
pub fn drop_audit_update_trigger(dialect: Dialect) -> String {
    drop_trigger(dialect, AUDIT_UPDATE)
}

/// Both guards, for schema creation.
pub fn audit_log_immutability(dialect: Dialect) -> Vec<String> {
    let mut all = audit_update_trigger(dialect);
    all.extend(audit_delete_trigger(dialect));
    all
}

/// Runs a DDL statement, expanding the tokens above for the pool's dialect.
///
/// Uses `sqlx::raw_sql`, the simple query protocol, rather than `sqlx::query`.
/// Two reasons, and the second is not optional: DDL carries no bind parameters,
/// so a prepared statement buys nothing; and PostgreSQL refuses more than one
/// command in a prepared statement ("cannot insert multiple commands into a
/// prepared statement"), while four of this project's DDL blocks bundle a
/// `CREATE TABLE` with its indexes in a single string. SQLite tolerated that;
/// Postgres does not, and a live migration test caught it.
///
/// Errors are the caller's to handle: `create_tables` propagates them with `?`,
/// while the best-effort `ALTER TABLE ... ADD COLUMN` statements discard them
/// with `let _ =` exactly as they did on SQLite alone.
#[macro_export]
macro_rules! db_ddl {
    ($pool:expr, $sql:expr) => {{
        let __pool = $pool;
        let __ddl_src = $sql;
        let __rendered = $crate::db::ddl::render(__ddl_src, __pool.dialect());
        match __pool {
            $crate::db::DbPool::Sqlite(__p) => {
                sqlx::raw_sql(&__rendered).execute(__p).await.map(|_| ())
            }
            $crate::db::DbPool::Postgres(__p) => {
                sqlx::raw_sql(&__rendered).execute(__p).await.map(|_| ())
            }
        }
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    #[test]
    fn test_sql_without_tokens_is_borrowed() {
        let sql = "CREATE TABLE IF NOT EXISTS t (a TEXT NOT NULL)";
        assert!(matches!(render(sql, Dialect::Postgres), Cow::Borrowed(_)));
    }

    #[test]
    fn test_the_primary_key_token_expands_per_dialect() {
        let sql = format!("CREATE TABLE t (id {PK}, a TEXT)");
        assert_eq!(
            render(&sql, Dialect::Sqlite),
            "CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, a TEXT)"
        );
        assert_eq!(
            render(&sql, Dialect::Postgres),
            "CREATE TABLE t (id BIGSERIAL PRIMARY KEY, a TEXT)"
        );
    }

    #[test]
    fn test_the_now_text_token_casts_on_postgres_only() {
        let sql = format!("CREATE TABLE t (created_at TEXT NOT NULL DEFAULT {NOW_TEXT})");
        assert!(render(&sql, Dialect::Sqlite).contains("(datetime('now'))"));
        assert!(render(&sql, Dialect::Postgres).contains("(NOW()::text)"));
    }

    /// Every token must be expanded by every dialect. An unexpanded `{{` would
    /// otherwise reach the database, and while that fails loudly, failing at
    /// boot is a poor substitute for failing in CI.
    #[test]
    fn test_no_token_survives_rendering_in_either_dialect() {
        let sql = format!("CREATE TABLE t (id {PK}, created_at TEXT DEFAULT {NOW_TEXT})");
        for dialect in [Dialect::Sqlite, Dialect::Postgres] {
            let rendered = render(&sql, dialect);
            assert!(
                !rendered.contains("{{"),
                "{dialect:?} left a token unexpanded: {rendered}"
            );
        }
    }

    /// The portability claims this module rests on, checked against a real
    /// SQLite rather than asserted in a comment: `BOOLEAN ... DEFAULT FALSE`
    /// and `TIMESTAMP DEFAULT CURRENT_TIMESTAMP` are written once and must be
    /// accepted by SQLite as-is. If SQLite rejected `FALSE`, the schema would
    /// need a third token.
    #[tokio::test]
    async fn test_sqlite_accepts_the_portable_spellings_used_in_the_schema() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        let sql = render(
            &format!(
                "CREATE TABLE portable (
                     id {PK},
                     flag_off BOOLEAN NOT NULL DEFAULT FALSE,
                     flag_on BOOLEAN NOT NULL DEFAULT TRUE,
                     created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                 )"
            ),
            Dialect::Sqlite,
        )
        .into_owned();
        crate::db_execute!(&pool, &sql).unwrap();

        crate::db_execute!(&pool, "INSERT INTO portable (id) VALUES (1)").unwrap();

        let (off, on): (bool, bool) = crate::db_fetch_one!(
            &pool,
            _,
            "SELECT flag_off, flag_on FROM portable WHERE id = 1"
        )
        .unwrap();
        assert!(!off, "DEFAULT FALSE must store false on SQLite");
        assert!(on, "DEFAULT TRUE must store true on SQLite");
    }

    /// Why the schema says `BIGINT` and never `INTEGER`.
    ///
    /// On PostgreSQL `INTEGER` is `int4`, which does not decode into the `i64`
    /// every model uses — a live migration test caught this as
    /// `mismatched types; Rust type i64 (as SQL type INT8) is not compatible
    /// with SQL type INT4`. It also caps `amount_minor` at about 2.1 billion
    /// minor units. On SQLite `BIGINT` carries INTEGER affinity, so widening
    /// costs nothing here; this asserts that, and that a value beyond int4's
    /// range survives the round trip.
    #[tokio::test]
    async fn test_sqlite_round_trips_a_bigint_beyond_int4_range() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        crate::db_execute!(&pool, "CREATE TABLE money (amount_minor BIGINT NOT NULL)").unwrap();

        let beyond_int4: i64 = 9_000_000_000;
        crate::db_execute!(
            &pool,
            "INSERT INTO money (amount_minor) VALUES (?)",
            beyond_int4
        )
        .unwrap();

        let read: i64 = crate::db_scalar_one!(&pool, _, "SELECT amount_minor FROM money").unwrap();
        assert_eq!(read, beyond_int4);
    }

    /// Why the schema says `TIMESTAMPTZ` and never bare `TIMESTAMP`.
    ///
    /// PostgreSQL's `TIMESTAMP` is "without time zone" and will not decode into
    /// the `DateTime<Utc>` the models use — a live repository round-trip caught
    /// this as `Rust type Option<DateTime<Utc>> (as SQL type TIMESTAMPTZ) is
    /// not compatible with SQL type TIMESTAMP`. SQLite gives both spellings the
    /// same NUMERIC affinity, so this asserts it round-trips there unchanged.
    #[tokio::test]
    async fn test_sqlite_round_trips_a_timestamptz_into_datetime_utc() {
        use chrono::{DateTime, TimeZone, Utc};

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE stamped (at TIMESTAMPTZ NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();

        let when: DateTime<Utc> = Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap();
        sqlx::query("INSERT INTO stamped (at) VALUES (?)")
            .bind(when)
            .execute(&pool)
            .await
            .unwrap();

        let read: DateTime<Utc> = sqlx::query_scalar("SELECT at FROM stamped")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(read, when);
    }

    /// The SQLite PK token must actually auto-assign, which is the only reason
    /// it is not just `INTEGER PRIMARY KEY`.
    #[tokio::test]
    async fn test_the_sqlite_primary_key_token_auto_assigns() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        let sql = render(
            &format!("CREATE TABLE t (id {PK}, a TEXT)"),
            Dialect::Sqlite,
        )
        .into_owned();
        crate::db_execute!(&pool, &sql).unwrap();

        let id: i64 =
            crate::db_scalar_one!(&pool, _, "INSERT INTO t (a) VALUES ('x') RETURNING id").unwrap();
        assert_eq!(id, 1);
    }

    #[test]
    fn test_postgres_needs_more_statements_than_sqlite_for_the_audit_triggers() {
        let sqlite = audit_log_immutability(Dialect::Sqlite);
        let postgres = audit_log_immutability(Dialect::Postgres);
        assert_eq!(sqlite.len(), 2);
        assert_eq!(postgres.len(), 6);

        // Postgres has no CREATE TRIGGER IF NOT EXISTS, so re-running the
        // migration must not fail on an existing trigger.
        assert!(
            postgres
                .iter()
                .filter(|s| s.contains("DROP TRIGGER IF EXISTS"))
                .count()
                == 2,
            "each postgres trigger must be dropped before being recreated"
        );
    }

    /// Both dialects must guard both operations. Losing one silently would make
    /// the audit log mutable on that database only.
    #[test]
    fn test_both_dialects_guard_update_and_delete() {
        for dialect in [Dialect::Sqlite, Dialect::Postgres] {
            let joined = audit_log_immutability(dialect).join(" ");
            assert!(
                joined.contains("prevent_portfolio_audit_update"),
                "{dialect:?} must guard UPDATE"
            );
            assert!(
                joined.contains("prevent_portfolio_audit_delete"),
                "{dialect:?} must guard DELETE"
            );
        }
    }

    /// Proves the SQLite trigger really aborts, rather than merely being
    /// created. `accounts/merge.rs` depends on this firing.
    #[tokio::test]
    async fn test_the_sqlite_audit_triggers_actually_refuse_writes() {
        let pool = crate::db::DbPool::Sqlite(SqlitePool::connect("sqlite::memory:").await.unwrap());
        let create = render(
            &format!("CREATE TABLE portfolio_audit_log (id {PK}, owner_id TEXT NOT NULL)"),
            Dialect::Sqlite,
        )
        .into_owned();
        crate::db_execute!(&pool, &create).unwrap();
        for statement in audit_log_immutability(Dialect::Sqlite) {
            crate::db_execute!(&pool, &statement).unwrap();
        }

        crate::db_execute!(
            &pool,
            "INSERT INTO portfolio_audit_log (owner_id) VALUES ('acct_a')"
        )
        .unwrap();

        let updated =
            crate::db_execute!(&pool, "UPDATE portfolio_audit_log SET owner_id = 'acct_b'");
        assert!(updated.is_err(), "an audit row must not be updatable");

        let deleted = crate::db_execute!(&pool, "DELETE FROM portfolio_audit_log");
        assert!(deleted.is_err(), "an audit row must not be deletable");
    }
}
