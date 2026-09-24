//! SQL is still written in `?` style during the migration to `query!`; this
//! rewrites it to `$1..$n`. Deleted with the macros once every repository
//! uses `query!`.

use std::borrow::Cow;

/// Rewrites `?` placeholders into PostgreSQL's `$1..$n` form.
///
/// SQL with no `?` is returned borrowed and untouched, so the common path
/// allocates nothing.
///
/// A `?` is only a placeholder when it sits in SQL proper. Inside a string
/// literal, a quoted identifier, a line comment or a block comment it is data,
/// and rewriting it there would corrupt the statement *silently* — the query
/// would still run, just wrongly. Hence the scanner rather than a
/// `str::replace`.
pub fn render_placeholders(sql: &str) -> Cow<'_, str> {
    if !sql.contains('?') {
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
    fn test_postgres_numbers_placeholders_from_one() {
        assert_eq!(
            render_placeholders("SELECT * FROM todos WHERE owner_id = ? AND completed = ?"),
            "SELECT * FROM todos WHERE owner_id = $1 AND completed = $2"
        );
    }

    #[test]
    fn test_postgres_sql_without_placeholders_is_borrowed() {
        let sql = "SELECT COUNT(*) FROM todos";
        let rendered = render_placeholders(sql);
        assert_eq!(rendered, sql);
        assert!(matches!(rendered, Cow::Borrowed(_)));
    }

    /// The whole reason this is a scanner and not `str::replace`. A `?` inside
    /// a literal is data; renumbering it would corrupt the statement *without*
    /// raising an error, which is the worst possible failure mode.
    #[test]
    fn test_a_question_mark_inside_a_string_literal_is_left_alone() {
        assert_eq!(
            render_placeholders("SELECT ? WHERE note = 'really?' AND x = ?"),
            "SELECT $1 WHERE note = 'really?' AND x = $2"
        );
    }

    #[test]
    fn test_an_escaped_quote_does_not_end_the_literal_early() {
        assert_eq!(
            render_placeholders("UPDATE t SET a = 'it''s a ?' WHERE id = ?"),
            "UPDATE t SET a = 'it''s a ?' WHERE id = $1"
        );
    }

    #[test]
    fn test_a_question_mark_in_a_quoted_identifier_is_left_alone() {
        assert_eq!(
            render_placeholders(r#"SELECT "od?d" FROM t WHERE id = ?"#),
            r#"SELECT "od?d" FROM t WHERE id = $1"#
        );
    }

    #[test]
    fn test_question_marks_in_comments_are_left_alone() {
        assert_eq!(
            render_placeholders("SELECT a -- why?\nFROM t WHERE id = ?"),
            "SELECT a -- why?\nFROM t WHERE id = $1"
        );
        assert_eq!(
            render_placeholders("SELECT a /* why? */ FROM t WHERE id = ?"),
            "SELECT a /* why? */ FROM t WHERE id = $1"
        );
    }

    #[test]
    fn test_placeholders_keep_counting_past_nine() {
        let sql = format!("INSERT INTO t VALUES ({})", vec!["?"; 11].join(", "));
        assert_eq!(
            render_placeholders(&sql),
            "INSERT INTO t VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"
        );
    }
}
