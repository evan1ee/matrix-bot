//! Pure Markdown derivations for notes: plain text for search and embedding,
//! excerpts, content hashes and embedding chunks.
//!
//! Everything a note stores besides its Markdown is derived here on write and
//! can be rebuilt from the Markdown alone. `pulldown-cmark` lives in the
//! application layer rather than the domain, which may only import std and a
//! few small crates.

use pulldown_cmark::{Event, Options, Parser, TagEnd};
use sha2::{Digest, Sha256};

use crate::domain::knowledge::note::ContentHash;
use crate::domain::knowledge::search::chunk_text;

/// Characters kept in a note's stored excerpt.
pub const EXCERPT_CHARS: usize = 200;

const CHUNK_CHARS: usize = 500;
const CHUNK_OVERLAP: usize = 100;

/// Plain text of `md`: text, code, link text, image alt text and table cells
/// are kept; syntax, HTML and URLs are dropped. Blocks are separated by `\n`.
pub fn to_plain_text(md: &str) -> String {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut out = String::with_capacity(md.len());
    for event in Parser::new_ext(md, options) {
        match event {
            Event::Text(text) | Event::Code(text) => out.push_str(&text),
            Event::SoftBreak => out.push(' '),
            Event::HardBreak | Event::Rule => out.push('\n'),
            Event::End(TagEnd::TableCell) => out.push(' '),
            Event::End(end) if ends_block(&end) => out.push('\n'),
            // Raw HTML (tags and whole blocks), footnote markers and task
            // checkboxes carry no searchable text.
            _ => {}
        }
    }
    out.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn ends_block(end: &TagEnd) -> bool {
    matches!(
        end,
        TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::BlockQuote(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::List(_)
            | TagEnd::Item
            | TagEnd::TableHead
            | TagEnd::TableRow
    )
}

/// `plain` with whitespace collapsed, cut to `max_chars` characters on a
/// character boundary, with `…` appended when it was cut.
pub fn excerpt(plain: &str, max_chars: usize) -> String {
    let collapsed = plain.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_chars {
        return collapsed;
    }
    let cut: String = collapsed.chars().take(max_chars).collect();
    format!("{}…", cut.trim_end())
}

/// Hex SHA-256 of the Markdown body.
pub fn content_hash(content: &str) -> ContentHash {
    let hex: String = Sha256::digest(content.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    // A SHA-256 digest is always 64 lowercase hex characters, which
    // `from_hex` accepts, so the error arm cannot be reached.
    match ContentHash::from_hex(&hex) {
        Ok(hash) => hash,
        Err(_) => unreachable!("sha256 hex is 64 lowercase characters"),
    }
}

/// Everything a note stores besides its Markdown, computed in one place so no
/// caller derives a field itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derived {
    pub content_hash: ContentHash,
    pub search_text: String,
    pub excerpt: String,
}

pub fn derive(markdown: &str) -> Derived {
    let search_text = to_plain_text(markdown);
    Derived {
        content_hash: content_hash(markdown),
        excerpt: excerpt(&search_text, EXCERPT_CHARS),
        search_text,
    }
}

/// Embedding chunks for a note: the title and plain text, split into
/// overlapping windows. Empty when the note has no plain text.
pub fn chunks(title: &str, plain: &str) -> Vec<String> {
    if plain.trim().is_empty() {
        return Vec::new();
    }
    chunk_text(&format!("{title}\n\n{plain}"), CHUNK_CHARS, CHUNK_OVERLAP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_drops_heading_and_emphasis_syntax() {
        assert_eq!(
            to_plain_text("# Title\n\nSome **bold** and _italic_ text"),
            "Title\nSome bold and italic text"
        );
    }

    #[test]
    fn plain_text_keeps_link_text_and_image_alt_but_not_urls() {
        let plain = to_plain_text("See [the Rust site](https://rust-lang.org) ![a diagram](d.png)");
        assert_eq!(plain, "See the Rust site a diagram");
        assert!(!plain.contains("rust-lang.org"));
        assert!(!plain.contains("d.png"));
    }

    #[test]
    fn plain_text_keeps_code_block_contents() {
        let plain = to_plain_text("Run:\n\n```rust\nfn main() {}\n```\n\nand `inline_code` too");
        assert!(plain.contains("fn main() {}"));
        assert!(plain.contains("inline_code"));
        assert!(!plain.contains("```"));
    }

    #[test]
    fn plain_text_keeps_table_cells() {
        let plain = to_plain_text("| name | role |\n|---|---|\n| Ada | author |\n");
        for cell in ["name", "role", "Ada", "author"] {
            assert!(plain.contains(cell), "missing {cell} in {plain:?}");
        }
        assert!(!plain.contains('|'));
        assert!(!plain.contains("---"));
    }

    #[test]
    fn plain_text_strips_html_tags_and_blocks() {
        assert_eq!(to_plain_text("text <b>bold</b> end"), "text bold end");
        let plain = to_plain_text("<div>\nhidden\n</div>\n\nvisible");
        assert!(plain.contains("visible"));
        assert!(!plain.contains('<'));
    }

    #[test]
    fn plain_text_preserves_cjk() {
        assert_eq!(to_plain_text("# 猫\n\n坐在**垫子**上"), "猫\n坐在垫子上");
    }

    #[test]
    fn plain_text_flattens_lists_and_quotes() {
        assert_eq!(
            to_plain_text("- one\n- two\n\n> quoted"),
            "one\ntwo\nquoted"
        );
    }

    #[test]
    fn plain_text_of_blank_markdown_is_empty() {
        assert_eq!(to_plain_text(""), "");
        assert_eq!(to_plain_text("  \n\n "), "");
    }

    #[test]
    fn excerpt_collapses_whitespace_and_leaves_short_text_alone() {
        assert_eq!(excerpt("a  b\n\n c", 200), "a b c");
        assert_eq!(excerpt("", 10), "");
    }

    #[test]
    fn excerpt_cuts_on_a_character_boundary() {
        assert_eq!(excerpt("猫猫猫猫", 2), "猫猫…");
        assert_eq!(excerpt("🦀🦀🦀", 3), "🦀🦀🦀");
        assert_eq!(excerpt("abcdef", 3), "abc…");
        assert_eq!(excerpt("abc def", 4), "abc…");
    }

    #[test]
    fn content_hash_is_the_hex_sha256_of_the_body() {
        assert_eq!(
            content_hash("abc").as_str(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            content_hash("").as_str(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_ne!(content_hash("a"), content_hash("a "));
    }

    #[test]
    fn chunks_prefix_the_title_and_split_long_text() {
        let short = chunks("Rust", "ownership and borrowing");
        assert_eq!(short, vec!["Rust\n\nownership and borrowing".to_string()]);

        let long_text = "word ".repeat(400);
        let long = chunks("Rust", &long_text);
        assert!(long.len() > 1);
        assert!(long[0].starts_with("Rust\n\n"));
        assert!(long.iter().all(|c| c.chars().count() <= 500));
    }

    #[test]
    fn chunks_of_empty_plain_text_is_empty() {
        assert!(chunks("Title", "   ").is_empty());
    }

    #[test]
    fn derive_computes_hash_plain_text_and_excerpt_from_the_markdown() {
        let md = "# Hi\n\n**bold** text";
        let derived = derive(md);
        assert_eq!(derived.content_hash, content_hash(md));
        assert_eq!(derived.search_text, "Hi\nbold text");
        assert_eq!(derived.excerpt, "Hi bold text");
    }

    #[test]
    fn derive_caps_the_excerpt_but_not_the_search_text() {
        let md = "word ".repeat(200);
        let derived = derive(&md);
        assert!(derived.excerpt.chars().count() <= EXCERPT_CHARS + 1);
        assert!(derived.excerpt.ends_with('…'));
        assert!(derived.search_text.chars().count() > EXCERPT_CHARS);
    }
}
