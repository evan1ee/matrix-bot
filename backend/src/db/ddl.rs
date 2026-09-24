//! The handful of DDL constructs that have no portable spelling.
//!
//! Most of this project's schema is already portable and stays written once:
//! `TEXT`, `INTEGER`, `BOOLEAN`, `TIMESTAMP DEFAULT CURRENT_TIMESTAMP`,
//! `CREATE TABLE IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS` and `UNIQUE`
//! all mean the same thing to PostgreSQL. The best-effort
//! `ALTER TABLE ... ADD COLUMN` statements need nothing either: their errors
//! are already discarded with `let _ =`.
//!
//! What is left is substituted through **named tokens** rather than by
//! pattern-matching the SQL. That is the whole point: a regex translator fails
//! silently when it meets a spelling it does not know, whereas an unsubstituted
//! `{{TOKEN}}` is not valid SQL and fails loudly at migration time, which is
//! boot. Adding a construct means adding a token here, not hoping a rule
//! generalises.

use std::borrow::Cow;

/// An auto-assigning integer primary key.
pub const PK: &str = "{{PK}}";

/// The current timestamp as a *default expression*.
///
/// Bare `CURRENT_TIMESTAMP` is portable and is used directly throughout the
/// schema. This token exists for the columns declared `TEXT` that need the
/// current timestamp: Postgres will not put a timestamp in a text column
/// without an explicit cast.
pub const NOW_TEXT: &str = "{{NOW_TEXT}}";

/// Expands the tokens above.
///
/// SQL containing no token is returned borrowed, so the common statement
/// allocates nothing.
pub fn render(sql: &str) -> Cow<'_, str> {
    if !sql.contains("{{") {
        return Cow::Borrowed(sql);
    }

    Cow::Owned(
        sql.replace(PK, "BIGSERIAL PRIMARY KEY")
            .replace(NOW_TEXT, "(NOW()::text)"),
    )
}

/// The name of the trigger guarding the audit log against UPDATE.
const AUDIT_UPDATE: &str = "prevent_portfolio_audit_update";
/// The name of the trigger guarding the audit log against DELETE.
const AUDIT_DELETE: &str = "prevent_portfolio_audit_delete";

/// Statements creating one audit-log guard trigger, idempotently.
///
/// PostgreSQL has no `CREATE TRIGGER IF NOT EXISTS` and no statement trigger
/// that can abort with a message: it needs a `plpgsql` function plus a row
/// trigger, and re-running the migration has to `DROP TRIGGER IF EXISTS`
/// first — hence three statements for one guard.
///
/// **Load-bearing for `accounts/merge.rs`.** That module relies on the UPDATE
/// trigger actually firing: `accounts::merge::move_rows` depends on it to
/// keep the audit log append-only across a merge.
fn guard_trigger(name: &str, event: &str, message: &str) -> Vec<String> {
    vec![
        format!(
            "CREATE OR REPLACE FUNCTION {name}() RETURNS trigger AS $fn$ \
             BEGIN RAISE EXCEPTION '{message}'; END; $fn$ LANGUAGE plpgsql;"
        ),
        drop_trigger(name),
        format!(
            "CREATE TRIGGER {name} BEFORE {event} ON portfolio_audit_log \
             FOR EACH ROW EXECUTE FUNCTION {name}();"
        ),
    ]
}

/// Drops one audit-log guard trigger if present.
///
/// `modules::portfolio::audit::absorb_ledger` drops the UPDATE guard to move a
/// ledger between owners, so this cannot live only in the schema module.
pub fn drop_trigger(name: &str) -> String {
    format!("DROP TRIGGER IF EXISTS {name} ON portfolio_audit_log;")
}

/// Creates the UPDATE guard. Also used to *restore* it after `absorb_ledger`.
pub fn audit_update_trigger() -> Vec<String> {
    guard_trigger(
        AUDIT_UPDATE,
        "UPDATE",
        "Audit log entries are immutable and cannot be updated",
    )
}

/// Creates the DELETE guard.
pub fn audit_delete_trigger() -> Vec<String> {
    guard_trigger(
        AUDIT_DELETE,
        "DELETE",
        "Audit log entries cannot be deleted",
    )
}

/// Drops the UPDATE guard, by name, for `absorb_ledger`.
pub fn drop_audit_update_trigger() -> String {
    drop_trigger(AUDIT_UPDATE)
}

/// Both guards, for schema creation.
pub fn audit_log_immutability() -> Vec<String> {
    let mut all = audit_update_trigger();
    all.extend(audit_delete_trigger());
    all
}

/// Runs a DDL statement, expanding the tokens above.
///
/// Uses `sqlx::raw_sql`, the simple query protocol, rather than `sqlx::query`.
/// Two reasons, and the second is not optional: DDL carries no bind parameters,
/// so a prepared statement buys nothing; and PostgreSQL refuses more than one
/// command in a prepared statement ("cannot insert multiple commands into a
/// prepared statement"), while four of this project's DDL blocks bundle a
/// `CREATE TABLE` with its indexes in a single string.
///
/// Errors are the caller's to handle: `create_tables` propagates them with `?`,
/// while the best-effort `ALTER TABLE ... ADD COLUMN` statements discard them
/// with `let _ =`.
#[macro_export]
macro_rules! db_ddl {
    ($pool:expr, $sql:expr) => {{
        let __p: &$crate::db::DbPool = $pool;
        let __ddl_src = $sql;
        let __rendered = $crate::db::ddl::render(__ddl_src);
        sqlx::raw_sql(&__rendered).execute(__p).await.map(|_| ())
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sql_without_tokens_is_borrowed() {
        let sql = "CREATE TABLE IF NOT EXISTS t (a TEXT NOT NULL)";
        assert!(matches!(render(sql), Cow::Borrowed(_)));
    }

    #[test]
    fn test_the_primary_key_token_expands() {
        let sql = format!("CREATE TABLE t (id {PK}, a TEXT)");
        assert_eq!(
            render(&sql),
            "CREATE TABLE t (id BIGSERIAL PRIMARY KEY, a TEXT)"
        );
    }

    #[test]
    fn test_the_now_text_token_casts() {
        let sql = format!("CREATE TABLE t (created_at TEXT NOT NULL DEFAULT {NOW_TEXT})");
        assert!(render(&sql).contains("(NOW()::text)"));
    }

    /// Every token must be expanded. An unexpanded `{{` would otherwise reach
    /// the database, and while that fails loudly, failing at boot is a poor
    /// substitute for failing in CI.
    #[test]
    fn test_no_token_survives_rendering() {
        let sql = format!("CREATE TABLE t (id {PK}, created_at TEXT DEFAULT {NOW_TEXT})");
        let rendered = render(&sql);
        assert!(
            !rendered.contains("{{"),
            "left a token unexpanded: {rendered}"
        );
    }

    #[test]
    fn test_the_audit_triggers_need_three_statements_each() {
        let all = audit_log_immutability();
        assert_eq!(all.len(), 6);

        // Postgres has no CREATE TRIGGER IF NOT EXISTS, so re-running the
        // migration must not fail on an existing trigger.
        assert_eq!(
            all.iter()
                .filter(|s| s.contains("DROP TRIGGER IF EXISTS"))
                .count(),
            2,
            "each trigger must be dropped before being recreated"
        );
    }

    /// Both operations must be guarded. Losing one silently would make the
    /// audit log mutable.
    #[test]
    fn test_both_update_and_delete_are_guarded() {
        let joined = audit_log_immutability().join(" ");
        assert!(
            joined.contains("prevent_portfolio_audit_update"),
            "must guard UPDATE"
        );
        assert!(
            joined.contains("prevent_portfolio_audit_delete"),
            "must guard DELETE"
        );
    }
}
