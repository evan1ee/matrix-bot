//! The module HTML subset as Discord Markdown.
//!
//! Inputs are bot-generated HTML whose user values are already `html_escape`d
//! (see `render/html.rs`); the tag scanner is shared with the sanitizer. Text is
//! entity-decoded and then Markdown-escaped, so a user value can neither start
//! formatting nor form Discord's `<@id>` mention syntax.

use super::html::{parse_tag, safe_href};

/// A formatting element being rendered; its text collects in `buf` and is
/// wrapped when the element closes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Root,
    Bold,
    Italic,
    Underline,
    Code,
    Pre,
    Paragraph,
    Unordered,
    Ordered,
    Item,
    Quote,
    Link,
}

impl Kind {
    fn of(tag: &str) -> Option<Self> {
        Some(match tag {
            "b" | "strong" => Self::Bold,
            "i" | "em" => Self::Italic,
            "u" => Self::Underline,
            "code" => Self::Code,
            "pre" => Self::Pre,
            "p" => Self::Paragraph,
            "ul" => Self::Unordered,
            "ol" => Self::Ordered,
            "li" => Self::Item,
            "blockquote" => Self::Quote,
            "a" => Self::Link,
            _ => return None,
        })
    }
}

struct Frame {
    kind: Kind,
    buf: String,
    /// `a`: the target when it is a web link, else the text stands alone.
    href: Option<String>,
    /// `ol`: the number of items rendered so far.
    items: usize,
}

impl Frame {
    fn new(kind: Kind, href: Option<String>) -> Self {
        Self {
            kind,
            buf: String::new(),
            href,
            items: 0,
        }
    }
}

/// Renders the module HTML subset (`b strong i em u code pre br p ul ol li a
/// blockquote`) as Discord Markdown. Text is Markdown-escaped, entities are
/// decoded, unknown tags are dropped keeping their text. Never fails.
pub fn html_to_markdown(html: &str) -> String {
    let mut stack = vec![Frame::new(Kind::Root, None)];
    let mut rest = html;

    while let Some(at) = rest.find('<') {
        push_text(&mut stack, &rest[..at]);
        rest = &rest[at..];
        match parse_tag(rest) {
            Some((tag, len)) => {
                rest = &rest[len..];
                if tag.name == "br" {
                    if !tag.closing {
                        top(&mut stack).buf.push('\n');
                    }
                } else if let Some(kind) = Kind::of(&tag.name) {
                    if tag.closing {
                        close(&mut stack, kind);
                    } else {
                        let href = (kind == Kind::Link).then(|| web_href(&tag.attrs)).flatten();
                        stack.push(Frame::new(kind, href));
                    }
                }
            }
            None => {
                push_text(&mut stack, "<");
                rest = &rest[1..];
            }
        }
    }
    push_text(&mut stack, rest);
    while stack.len() > 1 {
        pop(&mut stack);
    }
    let root = stack.pop().map(|f| f.buf).unwrap_or_default();
    collapse_blank_lines(&root)
}

/// Escapes Discord Markdown metacharacters in text that must render literally.
/// `<` and `@` are escaped too, so text never forms a mention.
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(
            c,
            '\\' | '*'
                | '_'
                | '~'
                | '`'
                | '|'
                | '>'
                | '<'
                | '#'
                | '-'
                | '['
                | ']'
                | '('
                | ')'
                | '@'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// `http`/`https` only: Discord links nothing else.
fn web_href(attrs: &str) -> Option<String> {
    let href = safe_href(attrs)?;
    let lower = href.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://")).then(|| {
        decode_entities(&href)
            .replace(' ', "%20")
            .replace(')', "%29")
    })
}

fn top(stack: &mut [Frame]) -> &mut Frame {
    // The root frame is never popped before the end, so the stack is never empty.
    let last = stack.len() - 1;
    &mut stack[last]
}

/// Inside `code` or `pre` text is shown as typed: decoded but not escaped.
fn push_text(stack: &mut [Frame], raw: &str) {
    if raw.is_empty() {
        return;
    }
    let literal = stack
        .iter()
        .any(|f| matches!(f.kind, Kind::Code | Kind::Pre));
    let decoded = decode_entities(raw);
    let text = if literal {
        decoded
    } else {
        escape_markdown(&decoded)
    };
    top(stack).buf.push_str(&text);
}

/// Closes `kind` and anything opened inside it; a closer with no opener is dropped.
fn close(stack: &mut Vec<Frame>, kind: Kind) {
    if !stack.iter().skip(1).any(|f| f.kind == kind) {
        return;
    }
    while stack.len() > 1 {
        let closed = stack.last().map(|f| f.kind);
        pop(stack);
        if closed == Some(kind) {
            break;
        }
    }
}

fn pop(stack: &mut Vec<Frame>) {
    let Some(frame) = stack.pop() else {
        return;
    };
    let in_pre = stack.iter().any(|f| f.kind == Kind::Pre);
    let parent = top(stack);
    let rendered = match frame.kind {
        Kind::Root => frame.buf,
        Kind::Bold => wrap(&frame.buf, "**"),
        Kind::Italic => wrap(&frame.buf, "*"),
        Kind::Underline => wrap(&frame.buf, "__"),
        Kind::Code if in_pre => frame.buf,
        Kind::Code if frame.buf.contains('`') => format!("`` {} ``", frame.buf),
        Kind::Code => format!("`{}`", frame.buf),
        Kind::Pre => format!("\n```\n{}\n```\n", frame.buf.trim_matches('\n')),
        Kind::Paragraph => format!("\n\n{}\n\n", frame.buf.trim()),
        Kind::Unordered | Kind::Ordered => format!("\n{}\n", frame.buf.trim_matches('\n')),
        Kind::Item => {
            let marker = if parent.kind == Kind::Ordered {
                parent.items += 1;
                format!("{}. ", parent.items)
            } else {
                "- ".to_owned()
            };
            let body = frame.buf.trim().replace('\n', "\n  ");
            format!("\n{marker}{body}")
        }
        Kind::Quote => {
            let quoted: Vec<String> = frame
                .buf
                .trim()
                .lines()
                .map(|line| format!("> {line}"))
                .collect();
            format!("\n{}\n", quoted.join("\n"))
        }
        Kind::Link => match frame.href {
            Some(href) => format!("[{}]({href})", frame.buf),
            None => frame.buf,
        },
    };
    parent.buf.push_str(&rendered);
}

/// Emphasis around whitespace-only text renders the markers literally, so it is dropped.
fn wrap(text: &str, marker: &str) -> String {
    if text.trim().is_empty() {
        text.to_owned()
    } else {
        format!("{marker}{text}{marker}")
    }
}

/// Runs of three or more newlines become one blank line; the result is trimmed.
pub(super) fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut newlines = 0;
    for c in text.chars() {
        if c == '\n' {
            newlines += 1;
            if newlines <= 2 {
                out.push(c);
            }
        } else {
            newlines = 0;
            out.push(c);
        }
    }
    out.trim().to_owned()
}

/// Decodes `&lt; &gt; &amp; &quot; &#39; &apos; &nbsp;` and numeric references;
/// anything else is kept as written.
pub(super) fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let decoded = rest.find(';').filter(|end| *end <= 10).and_then(|end| {
            let c = match &rest[1..end] {
                "lt" => Some('<'),
                "gt" => Some('>'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                body => body.strip_prefix('#').and_then(|num| {
                    let code = match num.strip_prefix(['x', 'X']) {
                        Some(hex) => u32::from_str_radix(hex, 16).ok(),
                        None => num.parse().ok(),
                    };
                    code.and_then(char::from_u32)
                }),
            };
            c.map(|c| (c, end + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &rest[len..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_the_module_subset() {
        let cases = [
            ("<b>a</b> <i>b</i> <u>c</u>", "**a** *b* __c__"),
            ("<strong>a</strong><em>b</em>", "**a***b*"),
            ("a<br/>b<br>c", "a\nb\nc"),
            ("<p>one</p><p>two</p>", "one\n\ntwo"),
            ("<ul><li>x</li><li>y</li></ul>", "- x\n- y"),
            ("<ol><li>x</li><li>y</li></ol>", "1. x\n2. y"),
            ("<a href=\"https://x.y/a_b\">t</a>", "[t](https://x.y/a_b)"),
            ("<a href=\"javascript:alert(1)\">t</a>", "t"),
            ("<a href=\"mailto:a@b.c\">t</a>", "t"),
            ("<code>a*b</code>", "`a*b`"),
            ("<code>a`b</code>", "`` a`b ``"),
            ("<pre>x\n*y*</pre>", "```\nx\n*y*\n```"),
            ("<pre><code>let x = 1;</code></pre>", "```\nlet x = 1;\n```"),
            ("&lt;b&gt; &amp; *", "\\<b\\> & \\*"),
            ("<script>x</script>", "x"),
            ("@everyone", "\\@everyone"),
            ("<blockquote>a<br/>b</blockquote>", "> a\n> b"),
            ("<b>unclosed", "**unclosed**"),
            ("a</b>c", "ac"),
            ("1 < 2", "1 \\< 2"),
            ("&#39;q&#x27; &unknown;", "'q' &unknown;"),
            ("<ul><li>a<ul><li>b</li></ul></li></ul>", "- a\n  - b"),
            ("<p></p><p></p>x", "x"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(html_to_markdown(input), expected, "input: {input}");
        }
    }

    #[test]
    fn a_real_todo_list_renders_without_stray_tags() {
        let html = "📋 <b>Todo list</b><br/>\
                    • <code>#12</code> buy <i>milk</i> 🔴 <b>[HIGH]</b><br/>\
                    • <code>#13</code> call mum_and_dad";
        let md = html_to_markdown(html);
        assert!(!md.contains('<') || md.contains("\\<"), "{md}");
        assert!(md.contains("**Todo list**"), "{md}");
        assert!(md.contains("`#12`"), "{md}");
        assert!(md.contains("mum\\_and\\_dad"), "{md}");
        assert!(md.contains("**\\[HIGH\\]**"), "{md}");
    }

    #[test]
    fn escaping_covers_every_metacharacter() {
        assert_eq!(escape_markdown("*_~`|"), "\\*\\_\\~\\`\\|");
        assert_eq!(escape_markdown("<@123>"), "\\<\\@123\\>");
        assert_eq!(escape_markdown("plain 字"), "plain 字");
    }
}
