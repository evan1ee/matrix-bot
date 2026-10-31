//! RFC 5545 text primitives: escaping, line folding and timestamps.

use chrono::{DateTime, Utc};

/// Content lines longer than this many octets must be folded (RFC 5545 §3.1).
const MAX_LINE_OCTETS: usize = 75;

/// `YYYYMMDDTHHMMSSZ`, the UTC form calendar clients expect.
pub fn format_utc(at: DateTime<Utc>) -> String {
    at.format("%Y%m%dT%H%M%SZ").to_string()
}

/// Escapes a TEXT value: backslash, semicolon, comma and newlines.
pub fn escape_text(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            other => out.push(other),
        }
    }
    out
}

/// Folds one content line to at most 75 octets per physical line, joining the
/// pieces with `\r\n ` and never cutting inside a UTF-8 sequence.
pub fn fold_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + line.len() / MAX_LINE_OCTETS * 3);
    let mut octets = 0;
    for c in line.chars() {
        let width = c.len_utf8();
        // A continuation line spends one octet on its leading space.
        if octets + width > MAX_LINE_OCTETS {
            out.push_str("\r\n ");
            octets = 1;
        }
        out.push(c);
        octets += width;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn format_utc_is_basic_iso() {
        let at = Utc.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
        assert_eq!(format_utc(at), "20260304T050607Z");
    }

    #[test]
    fn escape_text_escapes_specials() {
        assert_eq!(escape_text("a,b;c\\d\ne"), "a\\,b\\;c\\\\d\\ne");
    }

    #[test]
    fn fold_line_never_exceeds_75_octets_or_splits_utf8() {
        let input = format!("SUMMARY:{}", "日本語😀".repeat(60));
        let folded = fold_line(&input);
        for physical in folded.split("\r\n") {
            assert!(physical.len() <= MAX_LINE_OCTETS, "{}", physical.len());
        }
        assert!(folded.contains("\r\n "));
        assert_eq!(folded.replace("\r\n ", ""), input);
    }

    #[test]
    fn short_lines_are_left_alone() {
        assert_eq!(fold_line("BEGIN:VCALENDAR"), "BEGIN:VCALENDAR");
    }
}
