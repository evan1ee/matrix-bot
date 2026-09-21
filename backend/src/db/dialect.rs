//! Which SQL dialect the process is talking, and the one textual difference
//! that cannot be papered over by a driver.
//!
//! `sqlx` does not rewrite placeholders: SQLite reads `?`, PostgreSQL reads
//! `$1..$n`. `sqlx::Any` does not solve this either — and in 0.7 it carries
//! only `Null, Bool, SmallInt, Integer, BigInt, Real, Double, Text, Blob`, so
//! every `DateTime<Utc>` column in `modules/*/models.rs` would have to regress
//! to `String`. `schema_todos.rs` documents that exact regression as a bug
//! already fixed once. So the project keeps two real drivers and writes its
//! SQL once, in SQLite's `?` style, rendering per dialect on the way out.
//!
//! `db/dialect_probe.rs` proves the assumption this rests on: one
//! `#[derive(sqlx::FromRow)]` model satisfies both `SqliteRow` and `PgRow`,
//! chrono columns included, so no model needs a second mapping.

use std::borrow::Cow;

/// The SQL dialect a pool speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    /// Which dialect a `DATABASE_URL` asks for.
    ///
    /// Anything not recognisably PostgreSQL is treated as SQLite, matching the
    /// previous `pool.rs` behaviour where a non-`sqlite://` URL still went to
    /// the SQLite driver. Being explicit here means an unreachable "unknown
    /// scheme" error path does not have to exist.
    pub fn from_url(database_url: &str) -> Self {
        if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
            Dialect::Postgres
        } else {
            Dialect::Sqlite
        }
    }
}

/// Rewrites `?` placeholders into the form `dialect` expects.
///
/// SQLite is returned borrowed and untouched — the SQL in this project is
/// already written in its style, so the common path allocates nothing.
///
/// A `?` is only a placeholder when it sits in SQL proper. Inside a string
/// literal, a quoted identifier, a line comment or a block comment it is data,
/// and rewriting it there would corrupt the statement *silently* — the query
/// would still run, just wrongly. Hence the scanner rather than a
/// `str::replace`.
pub fn render_placeholders(sql: &str, dialect: Dialect) -> Cow<'_, str> {
    if dialect == Dialect::Sqlite || !sql.contains('?') {
        return Cow::Borrowed(sql);
    }

    let mut out = String::with_capacity(sql.len() + 8);
    let mut next_index = 1u32;
    let mut chars = sql.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '?' => {
                out.push('$');
                out.push_str(&next_index.to_string());
                next_index += 1;
            }
            // A string literal or quoted identifier. A doubled quote inside one
            // is an escape and does not end it, which falls out naturally: the
            // closing quote ends the literal and the next one reopens it.
            '\'' | '"' => {
                out.push(ch);
                for inner in chars.by_ref() {
                    out.push(inner);
                    if inner == ch {
                        break;
                    }
                }
            }
            '-' if chars.peek() == Some(&'-') => {
                out.push(ch);
                for inner in chars.by_ref() {
                    out.push(inner);
                    if inner == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                out.push(ch);
                let mut prev = '\0';
                for inner in chars.by_ref() {
                    out.push(inner);
                    if prev == '*' && inner == '/' {
                        break;
                    }
                    prev = inner;
                }
            }
            _ => out.push(ch),
        }
    }

    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_is_returned_untouched_and_unallocated() {
        let sql = "SELECT * FROM todos WHERE owner_id = ? AND completed = ?";
        let rendered = render_placeholders(sql, Dialect::Sqlite);
        assert_eq!(rendered, sql);
        assert!(matches!(rendered, Cow::Borrowed(_)));
    }

    #[test]
    fn test_postgres_numbers_placeholders_from_one() {
        assert_eq!(
            render_placeholders(
                "SELECT * FROM todos WHERE owner_id = ? AND completed = ?",
                Dialect::Postgres
            ),
            "SELECT * FROM todos WHERE owner_id = $1 AND completed = $2"
        );
    }

    #[test]
    fn test_postgres_sql_without_placeholders_is_borrowed() {
        let sql = "SELECT COUNT(*) FROM todos";
        let rendered = render_placeholders(sql, Dialect::Postgres);
        assert_eq!(rendered, sql);
        assert!(matches!(rendered, Cow::Borrowed(_)));
    }

    /// The whole reason this is a scanner and not `str::replace`. A `?` inside
    /// a literal is data; renumbering it would corrupt the statement *without*
    /// raising an error, which is the worst possible failure mode.
    #[test]
    fn test_a_question_mark_inside_a_string_literal_is_left_alone() {
        assert_eq!(
            render_placeholders(
                "SELECT ? WHERE note = 'really?' AND x = ?",
                Dialect::Postgres
            ),
            "SELECT $1 WHERE note = 'really?' AND x = $2"
        );
    }

    #[test]
    fn test_an_escaped_quote_does_not_end_the_literal_early() {
        assert_eq!(
            render_placeholders(
                "UPDATE t SET a = 'it''s a ?' WHERE id = ?",
                Dialect::Postgres
            ),
            "UPDATE t SET a = 'it''s a ?' WHERE id = $1"
        );
    }

    #[test]
    fn test_a_question_mark_in_a_quoted_identifier_is_left_alone() {
        assert_eq!(
            render_placeholders(r#"SELECT "od?d" FROM t WHERE id = ?"#, Dialect::Postgres),
            r#"SELECT "od?d" FROM t WHERE id = $1"#
        );
    }

    #[test]
    fn test_question_marks_in_comments_are_left_alone() {
        assert_eq!(
            render_placeholders("SELECT a -- why?\nFROM t WHERE id = ?", Dialect::Postgres),
            "SELECT a -- why?\nFROM t WHERE id = $1"
        );
        assert_eq!(
            render_placeholders("SELECT a /* why? */ FROM t WHERE id = ?", Dialect::Postgres),
            "SELECT a /* why? */ FROM t WHERE id = $1"
        );
    }

    #[test]
    fn test_placeholders_keep_counting_past_nine() {
        let sql = format!("INSERT INTO t VALUES ({})", vec!["?"; 11].join(", "));
        assert_eq!(
            render_placeholders(&sql, Dialect::Postgres),
            "INSERT INTO t VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        );
    }

    #[test]
    fn test_dialect_is_read_from_the_url_scheme() {
        assert_eq!(
            Dialect::from_url("postgres://u:p@localhost/bot"),
            Dialect::Postgres
        );
        assert_eq!(
            Dialect::from_url("postgresql://u:p@localhost/bot"),
            Dialect::Postgres
        );
        assert_eq!(
            Dialect::from_url("sqlite://data/bot_data.db?mode=rwc"),
            Dialect::Sqlite
        );
    }

    /// The default `DATABASE_URL` carries `?mode=rwc`. It must not be read as
    /// Postgres, and nothing downstream may treat its `?` as a placeholder.
    #[test]
    fn test_a_sqlite_url_with_query_parameters_stays_sqlite() {
        assert_eq!(
            Dialect::from_url("sqlite://data/bot_data.db?mode=rwc&cache=shared"),
            Dialect::Sqlite
        );
    }
}
