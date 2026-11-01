//! Wiki-link extraction: `[[Note Title]]` and `[[Note Title|shown text]]`.
//!
//! Pure and synchronous. The scanner is hand-written because the rules (skip
//! fenced code and inline code, one line per link) are awkward in a regex and
//! it must stay linear on a 100,000-character body.

use super::note::MAX_TITLE_LEN;

/// Most distinct link targets one note records, so a pasted index page cannot
/// fan out into thousands of rows.
pub const MAX_LINKS_PER_NOTE: usize = 50;

/// One distinct link target found in a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    /// The title as written (trimmed); kept so a dangling link can offer
    /// "create note" with the author's own capitalisation.
    pub target_title: String,
}

impl WikiLink {
    /// The case-insensitive key titles are matched on.
    pub fn key(&self) -> String {
        title_key(&self.target_title)
    }
}

/// Case-insensitive matching key for a note title.
pub fn title_key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// Distinct link targets in `markdown`, in first-seen order, at most
/// [`MAX_LINKS_PER_NOTE`]. A link to `own_title` is dropped (a note cannot
/// link to itself); links inside fenced or inline code are ignored.
pub fn extract_links(markdown: &str, own_title: &str) -> Vec<WikiLink> {
    let own = title_key(own_title);
    let mut seen = std::collections::HashSet::new();
    let mut links = Vec::new();
    let mut fence: Option<char> = None;

    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = ['`', '~']
            .into_iter()
            .find(|c| trimmed.chars().take(3).filter(|x| x == c).count() == 3);
        match (fence, marker) {
            (None, Some(m)) => {
                fence = Some(m);
                continue;
            }
            (Some(open), Some(m)) if open == m => {
                fence = None;
                continue;
            }
            (Some(_), _) => continue,
            _ => {}
        }
        for title in line_targets(line) {
            let key = title_key(&title);
            if key == own || !seen.insert(key) {
                continue;
            }
            links.push(WikiLink {
                target_title: title,
            });
            if links.len() == MAX_LINKS_PER_NOTE {
                return links;
            }
        }
    }
    links
}

/// Targets on one line, with inline code spans removed first.
fn line_targets(line: &str) -> Vec<String> {
    let mut visible = String::with_capacity(line.len());
    let mut in_code = false;
    for c in line.chars() {
        if c == '`' {
            in_code = !in_code;
        } else if !in_code {
            visible.push(c);
        }
    }

    let mut targets = Vec::new();
    let mut rest = visible.as_str();
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("]]") else { break };
        let inner = &after[..end];
        // `[[a [[b]]` is not a link to "a [[b": restart at the inner opener.
        if let Some(inner_open) = inner.rfind("[[") {
            rest = &after[inner_open..];
            continue;
        }
        let target = inner.split('|').next().unwrap_or("").trim();
        if !target.is_empty()
            && target.chars().count() <= MAX_TITLE_LEN
            && !target.contains(['[', ']'])
        {
            targets.push(target.to_string());
        }
        rest = &after[end + 2..];
    }
    targets
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(md: &str) -> Vec<String> {
        extract_links(md, "Me")
            .into_iter()
            .map(|l| l.target_title)
            .collect()
    }

    #[test]
    fn extracts_plain_and_aliased_links() {
        assert_eq!(
            titles("See [[Alpha]] and [[Beta Note|the beta]]."),
            ["Alpha", "Beta Note"]
        );
    }

    #[test]
    fn trims_and_dedups_case_insensitively_keeping_the_first_spelling() {
        assert_eq!(titles("[[ Alpha ]] [[alpha]] [[ALPHA|x]]"), ["Alpha"]);
    }

    #[test]
    fn ignores_links_in_fenced_and_inline_code() {
        let md = "```\n[[In fence]]\n```\n`[[inline]]` [[Real]]\n~~~\n[[tilde]]\n~~~\n[[After]]";
        assert_eq!(titles(md), ["Real", "After"]);
    }

    #[test]
    fn drops_self_links() {
        assert_eq!(titles("[[me]] [[Other]]"), ["Other"]);
    }

    #[test]
    fn ignores_empty_unclosed_and_multiline_links() {
        assert!(titles("[[]] [[   |x]] [[unclosed").is_empty());
        assert!(titles("[[split\ntitle]]").is_empty());
    }

    #[test]
    fn a_nested_opener_links_the_inner_title() {
        assert_eq!(titles("[[a [[b]]"), ["b"]);
    }

    #[test]
    fn handles_punctuation_and_unicode_titles() {
        assert_eq!(
            titles("[[C++ & Rust: notes!]] [[垫子]] ([[Paren]])."),
            ["C++ & Rust: notes!", "垫子", "Paren"]
        );
    }

    #[test]
    fn caps_distinct_targets_and_overlong_titles() {
        let many: String = (0..80).map(|i| format!("[[n{i}]] ")).collect();
        assert_eq!(titles(&many).len(), MAX_LINKS_PER_NOTE);
        let long = format!("[[{}]]", "x".repeat(MAX_TITLE_LEN + 1));
        assert!(titles(&long).is_empty());
    }
}
