//! The module HTML subset as Telegram's HTML parse mode, and message splitting.
//!
//! Inputs are bot-generated HTML whose user values are already `html_escape`d (see
//! `render/html.rs`); the tag scanner is shared with the sanitizer. Telegram accepts
//! only a handful of tags and the entities `&lt; &gt; &amp; &quot;`, so text is
//! entity-decoded and then re-escaped, and tags outside the mapping are dropped.

use super::html::{parse_tag, safe_href};
use super::markdown::{collapse_blank_lines, decode_entities};

/// Telegram's limit per message, in visible characters.
pub const MESSAGE_MAX: usize = 4096;

/// Module HTML subset → Telegram HTML parse mode: `b strong`→`b`, `i em`→`i`, `u`, `s strike del`→`s`, `code`,
/// `pre`, `blockquote`, `a href` (http/https/mailto only, else text); `br`→`\n`;
/// `p`→paragraph separated by a blank line; `ul>li`→`• item`; `ol>li`→`1. item`;
/// unknown tags dropped keeping their text. Text keeps only `&lt; &gt; &amp; &quot;`
/// (other entities decoded to characters). Runs of 3+ newlines collapse to 2. Never fails.
pub fn html_to_telegram(html: &str) -> String {
    let mut r = Renderer::default();
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        r.text(&rest[..at]);
        rest = &rest[at..];
        match parse_tag(rest) {
            Some((tag, len)) => {
                r.tag(&tag.name, tag.closing, &tag.attrs);
                rest = &rest[len..];
            }
            None => {
                r.text("<");
                rest = &rest[1..];
            }
        }
    }
    r.text(rest);
    while let Some(name) = r.open.pop() {
        r.out.push_str(&format!("</{name}>"));
    }
    collapse_blank_lines(&r.out)
}

#[derive(Default)]
struct Renderer {
    out: String,
    /// Telegram tags currently open, innermost last.
    open: Vec<&'static str>,
    /// One entry per open list: `Some(n)` numbered (items so far), `None` bulleted.
    lists: Vec<Option<usize>>,
}

impl Renderer {
    fn text(&mut self, raw: &str) {
        escape_into(&decode_entities(raw), false, &mut self.out);
    }

    fn tag(&mut self, name: &str, closing: bool, attrs: &str) {
        match name {
            "br" if !closing => self.out.push('\n'),
            "p" => self.blank_line(),
            "ul" | "ol" if closing => {
                self.lists.pop();
                self.line_start();
            }
            "ul" | "ol" => {
                self.lists.push((name == "ol").then_some(0));
                self.line_start();
            }
            "li" if closing => self.line_start(),
            "li" => {
                self.line_start();
                match self.lists.last_mut() {
                    Some(Some(n)) => {
                        *n += 1;
                        self.out.push_str(&format!("{n}. "));
                    }
                    _ => self.out.push_str("• "),
                }
            }
            _ => self.format_tag(name, closing, attrs),
        }
    }

    fn format_tag(&mut self, name: &str, closing: bool, attrs: &str) {
        let tg = match name {
            "b" | "strong" => "b",
            "i" | "em" => "i",
            "u" => "u",
            "s" | "strike" | "del" => "s",
            "code" => "code",
            "pre" => "pre",
            "blockquote" => "blockquote",
            "a" => "a",
            _ => return,
        };
        if closing {
            if self.open.contains(&tg) {
                while let Some(top) = self.open.pop() {
                    self.out.push_str(&format!("</{top}>"));
                    if top == tg {
                        break;
                    }
                }
            }
            return;
        }
        // Telegram rejects entities inside `code`, and inside `pre` other than `code`.
        let allowed = match self.open.last() {
            Some(&"pre") => tg == "code",
            Some(&"code") => false,
            _ => !self.open.contains(&"pre"),
        };
        if !allowed {
            return;
        }
        if tg == "a" {
            let Some(href) = safe_href(attrs) else { return };
            self.out.push_str("<a href=\"");
            escape_into(&decode_entities(&href), true, &mut self.out);
            self.out.push_str("\">");
        } else {
            self.out.push_str(&format!("<{tg}>"));
        }
        self.open.push(tg);
    }

    /// Ensures the output ends a line (no-op at the start or after a newline).
    fn line_start(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    /// Ensures a blank line separates what follows from what precedes.
    fn blank_line(&mut self) {
        if self.out.is_empty() {
            return;
        }
        let have = self.out.chars().rev().take_while(|c| *c == '\n').count();
        for _ in have..2 {
            self.out.push('\n');
        }
    }
}

/// `&`, `<` and `>` always; `"` too inside an attribute value.
fn escape_into(text: &str, attr: bool, out: &mut String) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// One unit of rendered Telegram HTML: a tag (free), or a visible character or entity.
struct Token<'a> {
    text: &'a str,
    /// `Some((name, closing))` for a tag.
    tag: Option<(String, bool)>,
}

fn tokenize(rendered: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut rest = rendered;
    while let Some(c) = rest.chars().next() {
        let (len, tag) = match c {
            '<' => match parse_tag(rest) {
                Some((tag, len)) => (len, Some((tag.name, tag.closing))),
                None => (1, None),
            },
            // An entity (`&amp;`, `&#39;`) is one visible character and never split.
            '&' => match rest.find(';').filter(|end| (2..=10).contains(end)) {
                Some(end)
                    if rest[1..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '#') =>
                {
                    (end + 1, None)
                }
                _ => (1, None),
            },
            _ => (c.len_utf8(), None),
        };
        tokens.push(Token {
            text: &rest[..len],
            tag,
        });
        rest = &rest[len..];
    }
    tokens
}

/// Splits rendered Telegram HTML into messages of ≤ MESSAGE_MAX visible characters
/// (entities count as one), at line boundaries where possible, else on a char boundary
/// outside a tag or entity; tags open at a split are closed and reopened in the next part.
pub fn split_messages(rendered: &str) -> Vec<String> {
    let tokens = tokenize(rendered);
    let mut parts = Vec::new();
    // Open tags (name, opening text) carried into the next part.
    let mut carry: Vec<(String, &str)> = Vec::new();
    let mut start = 0;
    while start < tokens.len() {
        let mut stack = carry.clone();
        let mut visible = 0;
        let mut at = start;
        let mut last_line: Option<(usize, Vec<(String, &str)>)> = None;
        while let Some(token) = tokens.get(at) {
            match &token.tag {
                Some((name, true)) => {
                    if let Some(pos) = stack.iter().rposition(|(n, _)| n == name) {
                        stack.truncate(pos);
                    }
                }
                Some((name, false)) => stack.push((name.clone(), token.text)),
                None => {
                    if visible == MESSAGE_MAX {
                        break;
                    }
                    visible += 1;
                    if token.text == "\n" {
                        last_line = Some((at + 1, stack.clone()));
                    }
                }
            }
            at += 1;
        }
        if at < tokens.len() {
            if let Some((end, snapshot)) = last_line {
                at = end;
                stack = snapshot;
            }
        }
        let mut part: String = carry.iter().map(|(_, open)| *open).collect();
        part.extend(tokens[start..at].iter().map(|t| t.text));
        for (name, _) in stack.iter().rev() {
            part.push_str(&format!("</{name}>"));
        }
        parts.push(part);
        carry = stack;
        start = at;
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::AccountId;
    use crate::domain::todos::{TodoItem, TodoPriority};

    /// Visible characters of Telegram HTML: tags are free, an entity counts as one.
    fn visible_len(s: &str) -> usize {
        visible_text(s).chars().count()
    }

    /// Tags stripped, entities left as written but counted as one by `visible_len`
    /// callers through `visible_text` below.
    fn visible_text(s: &str) -> String {
        let mut out = String::new();
        let mut in_tag = false;
        let mut entity = false;
        for c in s.chars() {
            match c {
                '<' if !entity => in_tag = true,
                '>' if in_tag => in_tag = false,
                _ if in_tag => {}
                '&' => {
                    entity = true;
                    out.push('\u{1}');
                }
                ';' if entity => entity = false,
                _ if entity => {}
                _ => out.push(c),
            }
        }
        out
    }

    #[test]
    fn maps_the_module_subset() {
        let cases = [
            ("<b>a</b> <strong>b</strong>", "<b>a</b> <b>b</b>"),
            ("<i>a</i><em>b</em>", "<i>a</i><i>b</i>"),
            ("<u>a</u>", "<u>a</u>"),
            (
                "<del>a</del><s>b</s><strike>c</strike>",
                "<s>a</s><s>b</s><s>c</s>",
            ),
            ("<code>a</code>", "<code>a</code>"),
            ("<pre>x\ny</pre>", "<pre>x\ny</pre>"),
            ("<pre><code>x</code></pre>", "<pre><code>x</code></pre>"),
            ("<blockquote>q</blockquote>", "<blockquote>q</blockquote>"),
            ("a<br/>b<br>c", "a\nb\nc"),
            ("<p>one</p><p>two</p>", "one\n\ntwo"),
            ("<ul><li>x</li><li>y</li></ul>", "• x\n• y"),
            ("<ol><li>x</li><li>y</li></ol>", "1. x\n2. y"),
            (
                "<a href=\"https://x.y/?a=1&amp;b=2\">t</a>",
                "<a href=\"https://x.y/?a=1&amp;b=2\">t</a>",
            ),
            (
                "<a href=\"mailto:a@b.c\">t</a>",
                "<a href=\"mailto:a@b.c\">t</a>",
            ),
            ("<a href=\"javascript:alert(1)\">t</a>", "t"),
            ("<script>x</script>", "x"),
            ("<span>kept</span>", "kept"),
            ("a<br/><br/><br/><br/>b", "a\n\nb"),
            ("a&nbsp;&#39;q&#x27; &eacute;", "a 'q' &amp;eacute;"),
            ("&lt;b&gt; &amp; &quot;", "&lt;b&gt; &amp; \""),
            ("1 < 2 > 0", "1 &lt; 2 &gt; 0"),
            ("@everyone", "@everyone"),
            ("<b>unclosed", "<b>unclosed</b>"),
            ("a</b>c", "ac"),
            ("<pre><b>x</b></pre>", "<pre>x</pre>"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(html_to_telegram(input), expected, "input: {input}");
        }
    }

    #[test]
    fn a_real_todo_list_renders_without_stray_tags() {
        let item = |id, text: &str| TodoItem {
            id,
            room_id: "r".into(),
            owner_id: AccountId::from_seed("t"),
            sender_name: "Ann".into(),
            text: text.into(),
            priority: TodoPriority::High,
            due_at: None,
            remind_before_minutes: None,
            completed: false,
            created_at: None,
            tags: vec![],
            recurrence: crate::domain::reminders::RecurrencePattern::None,
            recurrence_anchor: None,
            next_todo_id: None,
        };
        let html = crate::interface::chat::todos::view::format_todo_list(
            "Room Tasks",
            &[item(1, "buy <milk>"), item(2, "call mum")],
            1,
            &chrono_tz::UTC,
            &Default::default(),
        );
        let out = html_to_telegram(&html);
        assert!(out.contains("<b>Room Tasks (Page 1):</b>"), "{out}");
        assert!(out.contains("buy &lt;milk&gt;"), "{out}");
        assert!(!out.contains("<br") && !out.contains("<del"), "{out}");
        assert!(out.contains("call mum"), "{out}");
    }

    #[test]
    fn short_text_is_one_part() {
        assert_eq!(split_messages("hello <b>x</b>"), vec!["hello <b>x</b>"]);
    }

    #[test]
    fn long_text_splits_at_lines_within_the_limit() {
        let input: String = (0..900).map(|i| format!("line {i:04}\n")).collect();
        assert!(input.chars().count() >= 9_000);
        let parts = split_messages(&input);
        assert_eq!(parts.len(), 3, "{}", parts.len());
        for p in &parts {
            assert!(visible_len(p) <= MESSAGE_MAX);
            assert!(p.ends_with('\n'), "split at a line boundary");
        }
        assert_eq!(parts.concat(), input);
    }

    #[test]
    fn a_single_long_line_splits_on_a_char_boundary() {
        let input = "字".repeat(5_000);
        let parts = split_messages(&input);
        assert_eq!(parts.len(), 2);
        assert_eq!(visible_len(&parts[0]), MESSAGE_MAX);
        assert_eq!(parts.concat(), input);
    }

    #[test]
    fn tags_open_across_a_split_are_closed_and_reopened() {
        let input = format!("<b><i>{}</i></b>", "a".repeat(5_000));
        let parts = split_messages(&input);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("<b><i>") && parts[0].ends_with("</i></b>"));
        assert!(parts[1].starts_with("<b><i>") && parts[1].ends_with("</i></b>"));
        assert_eq!(visible_text(&parts.concat()), "a".repeat(5_000));
    }

    #[test]
    fn strikethrough_open_across_a_split_is_closed_and_reopened() {
        let input = format!("<s>{}</s>", "a".repeat(5_000));
        let parts = split_messages(&input);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].starts_with("<s>") && parts[0].ends_with("</s>"));
        assert!(parts[1].starts_with("<s>") && parts[1].ends_with("</s>"));
        assert_eq!(visible_text(&parts.concat()), "a".repeat(5_000));
    }

    #[test]
    fn entities_are_never_split() {
        let input = "&amp;".repeat(5_000);
        let parts = split_messages(&input);
        assert_eq!(parts.len(), 2);
        for p in &parts {
            assert!(visible_len(p) <= MESSAGE_MAX);
            assert_eq!(p.matches('&').count(), p.matches(';').count());
            assert!(p.replace("&amp;", "").is_empty());
        }
        assert_eq!(parts.concat(), input);
    }
}
