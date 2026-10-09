//! The Markdown file a note exports to and imports from: YAML frontmatter with
//! a fixed key set, a blank line, then the note's Markdown byte for byte.

use chrono::SecondsFormat;
use serde::Deserialize;

use crate::domain::knowledge::note::NoteDocument;

/// A note file read back. Parsing never fails: a file with no usable
/// frontmatter is a note whose whole text is its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedNote {
    pub id: Option<i64>,
    pub title: String,
    pub markdown: String,
    pub tags: Vec<String>,
    /// As written in the file; the importer decides whether it is a known type.
    pub note_type: Option<String>,
    /// `None` when the file does not say, which leaves an existing note's state alone.
    pub archived: Option<bool>,
    pub source_ref: Option<String>,
}

/// Longest slug in an exported file name, in characters.
const MAX_SLUG_CHARS: usize = 60;

/// Keys that make a leading `---` block frontmatter. Without one of them the
/// block is more likely two horizontal rules around prose.
const KNOWN_KEYS: [&str; 5] = ["id", "type", "title", "tags", "source"];

#[derive(Deserialize)]
struct Frontmatter {
    id: Option<i64>,
    #[serde(rename = "type")]
    note_type: Option<String>,
    title: Option<String>,
    tags: Option<Vec<String>>,
    archived: Option<bool>,
    source_ref: Option<String>,
}

pub fn render_note_file(doc: &NoteDocument) -> String {
    let note = &doc.note;
    let tags: Vec<String> = note.tags.iter().map(|tag| quote(tag)).collect();
    // `source_ref` is the only optional key, so the key order stays fixed.
    let source_ref = note
        .source_ref
        .as_deref()
        .map(|r| {
            format!(
                "source_ref: {}
",
                quote(r)
            )
        })
        .unwrap_or_default();
    format!(
        "---
id: {}
type: {}
title: {}
tags: [{}]
source: {}
{}archived: {}
created_at: {}
updated_at: {}
---

{}",
        note.id,
        note.note_type,
        quote(&note.title),
        tags.join(", "),
        note.source_channel,
        source_ref,
        note.is_archived,
        note.created_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        note.updated_at.to_rfc3339_opts(SecondsFormat::Secs, true),
        doc.markdown,
    )
}

/// A YAML double-quoted scalar. JSON string syntax is valid YAML and escapes
/// every control character; the Unicode line separators JSON leaves raw are
/// escaped too, because some YAML readers fold them.
fn quote(value: &str) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "\"\"".to_string())
        .replace('\u{2028}', r"\u2028")
        .replace('\u{2029}', r"\u2029")
        .replace('\u{85}', r"\u0085")
}

pub fn parse_note_file(file_name: &str, text: &str) -> ParsedNote {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let framed = split_frontmatter(text)
        .and_then(|(yaml, body)| parse_frontmatter(yaml).map(|front| (front, body)));
    let (front, body) = match framed {
        Some((front, body)) => (Some(front), body),
        None => (None, text),
    };
    let (id, note_type, tags, title, archived, source_ref) = match front {
        Some(f) => (
            f.id,
            f.note_type,
            f.tags.unwrap_or_default(),
            f.title,
            f.archived,
            f.source_ref,
        ),
        None => (None, None, Vec::new(), None, None, None),
    };
    let title = title
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .or_else(|| first_heading(body))
        .unwrap_or_else(|| file_stem(file_name));
    ParsedNote {
        id,
        title,
        markdown: body.to_string(),
        tags,
        note_type,
        archived,
        source_ref,
    }
}

/// Splits `---\n<yaml>---\n[\n]<body>`. Exactly one blank line after the
/// closing fence belongs to the format, so a body that starts with newlines
/// round-trips.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            let body = &rest[offset + line.len()..];
            let body = body
                .strip_prefix("\r\n")
                .or_else(|| body.strip_prefix('\n'))
                .unwrap_or(body);
            return Some((&rest[..offset], body));
        }
        offset += line.len();
    }
    None
}

fn parse_frontmatter(yaml: &str) -> Option<Frontmatter> {
    let value: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).ok()?;
    let map = value.as_mapping()?;
    if !KNOWN_KEYS.iter().any(|key| map.contains_key(*key)) {
        return None;
    }
    serde_yaml_ng::from_value(value).ok()
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.strip_prefix("# "))
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
}

fn file_stem(file_name: &str) -> String {
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let stem = match name.len().checked_sub(3) {
        Some(at) if name.is_char_boundary(at) && name[at..].eq_ignore_ascii_case(".md") => {
            &name[..at]
        }
        _ => name,
    };
    stem.trim().to_string()
}

/// `<slug>-<id>`, the stem of an exported file name.
pub fn export_slug(title: &str, id: i64) -> String {
    let mut slug = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug
        .trim_matches('-')
        .chars()
        .take(MAX_SLUG_CHARS)
        .collect();
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        format!("note-{id}")
    } else {
        format!("{slug}-{id}")
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::application::knowledge::markdown;
    use crate::domain::accounts::AccountId;
    use crate::domain::knowledge::note::{Note, NoteId, NoteType};

    fn doc(title: &str, markdown_text: &str, tags: &[&str], note_type: NoteType) -> NoteDocument {
        let at = Utc.with_ymd_and_hms(2026, 10, 8, 2, 0, 0).unwrap();
        NoteDocument {
            note: Note {
                id: NoteId(42),
                owner_id: AccountId::from_seed("note_file"),
                title: title.to_string(),
                note_type,
                source_channel: "web".to_string(),
                source_ref: None,
                tags: tags.iter().map(|t| t.to_string()).collect(),
                excerpt: String::new(),
                content_hash: markdown::content_hash(markdown_text),
                is_archived: false,
                created_at: at,
                updated_at: at,
                indexed_at: None,
            },
            markdown: markdown_text.to_string(),
        }
    }

    #[test]
    fn a_rendered_file_has_a_fixed_key_order_and_the_body_after_a_blank_line() {
        let file = render_note_file(&doc(
            "Use Postgres",
            "# Why\n\nBoring.",
            &["a", "b"],
            NoteType::Decision,
        ));
        assert_eq!(
            file,
            "---\nid: 42\ntype: decision\ntitle: \"Use Postgres\"\ntags: [\"a\", \"b\"]\nsource: web\narchived: false\n\
             created_at: 2026-10-08T02:00:00Z\nupdated_at: 2026-10-08T02:00:00Z\n---\n\n# Why\n\nBoring."
        );
    }

    #[test]
    fn render_then_parse_returns_the_same_note() {
        let bodies = [
            "plain",
            "\n\nleading blank lines survive",
            "trailing newline\n",
            "windows\r\nline endings\r\n",
            "---\nlooks like frontmatter\n---\n",
            "猫坐在垫子上 🐈\n\n- 面粉",
        ];
        for body in bodies {
            let original = doc(
                "Title: with \"quotes\" and 猫 # hash",
                body,
                &["x", "y-z"],
                NoteType::Idea,
            );
            let parsed = parse_note_file("ignored.md", &render_note_file(&original));
            assert_eq!(parsed.markdown, body, "body {body:?}");
            assert_eq!(parsed.title, original.note.title);
            assert_eq!(parsed.tags, original.note.tags);
            assert_eq!(parsed.id, Some(42));
            assert_eq!(parsed.note_type.as_deref(), Some("idea"));
        }
    }

    #[test]
    fn archived_state_and_source_ref_round_trip() {
        let mut original = doc("Kept", "body", &[], NoteType::Note);
        original.note.is_archived = true;
        original.note.source_ref = Some("mxc://x/\"y\"".to_string());

        let parsed = parse_note_file("a.md", &render_note_file(&original));

        assert_eq!(parsed.archived, Some(true));
        assert_eq!(parsed.source_ref.as_deref(), Some("mxc://x/\"y\""));
        let plain = parse_note_file(
            "b.md",
            &render_note_file(&doc("T", "b", &[], NoteType::Note)),
        );
        assert_eq!(plain.archived, Some(false));
        assert_eq!(plain.source_ref, None);
        assert_eq!(parse_note_file("c.md", "# only a heading").archived, None);
    }

    #[test]
    fn a_title_with_a_newline_or_control_character_round_trips() {
        let original = doc("line one\nline\ttwo\u{7}", "body", &[], NoteType::Note);
        let parsed = parse_note_file("x.md", &render_note_file(&original));
        assert_eq!(parsed.title, "line one\nline\ttwo\u{7}");
        assert!(parsed.tags.is_empty());
    }

    #[test]
    fn a_file_without_frontmatter_takes_its_title_from_the_first_heading() {
        let parsed = parse_note_file("notes/ignored.md", "intro\n\n# The Real Title\nbody");
        assert_eq!(parsed.title, "The Real Title");
        assert_eq!(parsed.markdown, "intro\n\n# The Real Title\nbody");
        assert_eq!(parsed.id, None);
        assert_eq!(parsed.note_type, None);
    }

    #[test]
    fn a_file_with_no_heading_takes_its_title_from_the_file_stem() {
        let parsed = parse_note_file("vault/sub/Shopping list.md", "milk\neggs");
        assert_eq!(parsed.title, "Shopping list");
        let parsed = parse_note_file("plain", "text");
        assert_eq!(parsed.title, "plain");
    }

    #[test]
    fn frontmatter_without_a_title_falls_back_like_a_file_without_frontmatter() {
        let parsed = parse_note_file("a.md", "---\nid: 7\n---\n\n# From heading\ntext");
        assert_eq!(parsed.id, Some(7));
        assert_eq!(parsed.title, "From heading");
        assert_eq!(parsed.markdown, "# From heading\ntext");
    }

    #[test]
    fn unknown_keys_are_ignored_and_other_tools_dates_do_not_matter() {
        let parsed = parse_note_file(
            "a.md",
            "---\ntitle: T\naliases: [x]\ncreated: 2024-01-01\ncssclass: wide\n---\nbody",
        );
        assert_eq!(parsed.title, "T");
        assert_eq!(parsed.markdown, "body");
    }

    #[test]
    fn malformed_frontmatter_leaves_the_whole_text_as_the_body() {
        let text = "---\ntitle: [unclosed\n---\nbody";
        let parsed = parse_note_file("broken.md", text);
        assert_eq!(parsed.markdown, text);
        assert_eq!(parsed.title, "broken");
        assert_eq!(parsed.id, None);
    }

    #[test]
    fn a_horizontal_rule_pair_that_is_not_frontmatter_stays_in_the_body() {
        let text = "---\njust a sentence between two rules\n---\nrest";
        let parsed = parse_note_file("rule.md", text);
        assert_eq!(parsed.markdown, text);
    }

    #[test]
    fn a_byte_order_mark_and_crlf_frontmatter_are_accepted() {
        let parsed = parse_note_file(
            "a.md",
            "\u{feff}---\r\nid: 3\r\ntitle: T\r\n---\r\n\r\nbody",
        );
        assert_eq!(parsed.id, Some(3));
        assert_eq!(parsed.title, "T");
        assert_eq!(parsed.markdown, "body");
    }

    #[test]
    fn slugs_keep_cjk_drop_punctuation_and_stay_short() {
        assert_eq!(
            export_slug("Use PostgreSQL, again!", 42),
            "use-postgresql-again-42"
        );
        assert_eq!(export_slug("猫坐在垫子上", 7), "猫坐在垫子上-7");
        assert_eq!(export_slug("***", 9), "note-9");
        let long = export_slug(&"a".repeat(200), 5);
        assert_eq!(long, format!("{}-5", "a".repeat(60)));
        assert!(!export_slug("../../etc/passwd", 1).contains('/'));
    }
}
