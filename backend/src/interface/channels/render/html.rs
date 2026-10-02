//! The HTML subset sanitizer. Inputs are bot-generated HTML whose user values
//! are already `html_escape`d, so a small tag scanner is enough; this is a
//! allowlist filter for what a platform renderer receives, not a security
//! boundary for arbitrary HTML.

/// Tags kept verbatim (attributes dropped, except `href` on `a`).
const KEPT: [&str; 10] = [
    "b",
    "i",
    "u",
    "code",
    "pre",
    "p",
    "ul",
    "ol",
    "li",
    "blockquote",
];

/// One scanned tag; shared with the Markdown renderer so the crate has one HTML scanner.
pub(super) struct Tag {
    /// Lower-cased.
    pub(super) name: String,
    pub(super) closing: bool,
    pub(super) attrs: String,
}

/// Keeps only `b i u code pre br p ul ol li blockquote` and `a` with an
/// `http`, `https` or `mailto` href; drops other tags but keeps their text;
/// escapes stray `<`, `>` and `&` that are not part of an entity. Kept tags
/// left open are closed at the end and stray closing tags are dropped, so the
/// output is always balanced.
pub fn sanitize_subset(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut open: Vec<&'static str> = Vec::new();
    let mut rest = html;

    while let Some(at) = rest.find('<') {
        escape_into(&rest[..at], false, &mut out);
        rest = &rest[at..];
        match parse_tag(rest) {
            Some((tag, len)) => {
                emit(&tag, &mut open, &mut out);
                rest = &rest[len..];
            }
            None => {
                out.push_str("&lt;");
                rest = &rest[1..];
            }
        }
    }
    escape_into(rest, false, &mut out);

    while let Some(name) = open.pop() {
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    }
    out
}

fn emit(tag: &Tag, open: &mut Vec<&'static str>, out: &mut String) {
    let kept = |name: &str| KEPT.iter().copied().find(|k| *k == name);
    if tag.name == "br" {
        if !tag.closing {
            out.push_str("<br/>");
        }
    } else if tag.name == "a" {
        if tag.closing {
            close(open, "a", out);
        } else if let Some(href) = safe_href(&tag.attrs) {
            open.push("a");
            out.push_str("<a href=\"");
            escape_into(&href, true, out);
            out.push_str("\">");
        }
    } else if let Some(name) = kept(&tag.name) {
        if tag.closing {
            close(open, name, out);
        } else {
            open.push(name);
            out.push('<');
            out.push_str(name);
            out.push('>');
        }
    }
}

/// Closes `name` and anything opened inside it; a closer with no opener is dropped.
fn close(open: &mut Vec<&'static str>, name: &str, out: &mut String) {
    if !open.contains(&name) {
        return;
    }
    while let Some(top) = open.pop() {
        out.push_str("</");
        out.push_str(top);
        out.push('>');
        if top == name {
            break;
        }
    }
}

/// Parses a tag at the start of `s` (which begins with `<`); `None` when it is
/// not a tag (stray `<`, or no closing `>`). A `>` inside a quoted attribute
/// value does not end the tag. Returns the tag and its byte length.
pub(super) fn parse_tag(s: &str) -> Option<(Tag, usize)> {
    let body = &s[1..];
    let (closing, body) = match body.strip_prefix('/') {
        Some(b) => (true, b),
        None => (false, body),
    };
    if !body.starts_with(|c: char| c.is_ascii_alphabetic()) {
        return None;
    }
    let name_len = body
        .find(|c: char| !c.is_ascii_alphanumeric())
        .unwrap_or(body.len());
    let name = body[..name_len].to_ascii_lowercase();

    let mut quote: Option<char> = None;
    for (i, c) in body[name_len..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => {
                let attrs = body[name_len..name_len + i].to_owned();
                let consumed = s.len() - body.len() + name_len + i + 1;
                return Some((
                    Tag {
                        name,
                        closing,
                        attrs,
                    },
                    consumed,
                ));
            }
            (None, _) => {}
        }
    }
    None
}

/// The `href` value when its scheme is allowed.
pub(super) fn safe_href(attrs: &str) -> Option<String> {
    let value = attr_value(attrs, "href")?;
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    ["http://", "https://", "mailto:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme))
        .then(|| value.to_owned())
}

fn attr_value(attrs: &str, wanted: &str) -> Option<String> {
    let mut rest = attrs;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if rest.is_empty() {
            return None;
        }
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        rest = rest[name_end..].trim_start();
        let Some(after_eq) = rest.strip_prefix('=') else {
            continue;
        };
        let after_eq = after_eq.trim_start();
        let (value, tail) = match after_eq.chars().next() {
            Some(q @ ('"' | '\'')) => {
                let inner = &after_eq[1..];
                let end = inner.find(q).unwrap_or(inner.len());
                (&inner[..end], inner.get(end + 1..).unwrap_or(""))
            }
            _ => {
                let end = after_eq.find(char::is_whitespace).unwrap_or(after_eq.len());
                (&after_eq[..end], &after_eq[end..])
            }
        };
        if name.eq_ignore_ascii_case(wanted) {
            return Some(value.to_owned());
        }
        rest = tail;
    }
}

/// Escapes `<`, `>` and any `&` that does not start an entity; with `quote`
/// also `"` (attribute values).
fn escape_into(s: &str, quote: bool, out: &mut String) {
    for (i, c) in s.char_indices() {
        match c {
            '&' if starts_entity(&s[i..]) => out.push('&'),
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if quote => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// `&name;` / `&#123;` / `&#x1f;` with a short body.
fn starts_entity(s: &str) -> bool {
    let body = &s[1..];
    let len = body
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '#')
        .unwrap_or(body.len());
    (1..=10).contains(&len) && body[len..].starts_with(';')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subset_keeps_formatting_and_drops_everything_else() {
        assert_eq!(
            sanitize_subset("<b>a</b><script>x</script><span>c</span>"),
            "<b>a</b>xc"
        );
        assert_eq!(
            sanitize_subset("<a href=\"javascript:alert(1)\">x</a>"),
            "x"
        );
        assert_eq!(
            sanitize_subset("<a href=\"https://e.com\">x</a>"),
            "<a href=\"https://e.com\">x</a>"
        );
    }

    #[test]
    fn table_of_inputs_and_outputs() {
        let cases = [
            // nested lists survive intact
            (
                "<ul><li>a<ul><li>b</li></ul></li></ul>",
                "<ul><li>a<ul><li>b</li></ul></li></ul>",
            ),
            // unclosed allowed tags are closed so formatting cannot leak
            ("<b>bold", "<b>bold</b>"),
            ("<ul><li>a", "<ul><li>a</li></ul>"),
            // stray closing tags are dropped
            ("a</b>c", "ac"),
            // other attributes dropped, quotes of either kind handled, `>` inside a value
            (
                "<a title=\"x>y\" href='https://e.com/?a=1&amp;b=2'>x</a>",
                "<a href=\"https://e.com/?a=1&amp;b=2\">x</a>",
            ),
            ("<b class=\"big\" onclick=\"evil()\">x</b>", "<b>x</b>"),
            // raw ampersand in an href is escaped, a quote cannot break out
            (
                "<a href=\"https://e.com/?a=1&b=2\">x</a>",
                "<a href=\"https://e.com/?a=1&amp;b=2\">x</a>",
            ),
            (
                "<a href=\"https://e.com/&quot;onclick=1\">x</a>",
                "<a href=\"https://e.com/&quot;onclick=1\">x</a>",
            ),
            // entities preserved, stray specials escaped
            ("a &amp; b &lt;c&gt;", "a &amp; b &lt;c&gt;"),
            ("a & b", "a &amp; b"),
            ("1 < 2 > 0", "1 &lt; 2 &gt; 0"),
            ("trailing <", "trailing &lt;"),
            ("<b", "&lt;b"),
            // uppercase tag names and schemes
            ("<B>x</B>", "<b>x</b>"),
            (
                "<A HREF=\"HTTPS://E.com\">x</A>",
                "<a href=\"HTTPS://E.com\">x</a>",
            ),
            // schemes
            (
                "<a href=\"mailto:a@b.c\">x</a>",
                "<a href=\"mailto:a@b.c\">x</a>",
            ),
            (
                "<a href=\"http://e.com\">x</a>",
                "<a href=\"http://e.com\">x</a>",
            ),
            ("<a href=\"data:text/html,x\">x</a>", "x"),
            ("<a href=\" javascript:x\">x</a>", "x"),
            ("<a href=\"//e.com\">x</a>", "x"),
            ("<a>x</a>", "x"),
            // void and self-closing br
            ("a<br/>b<br>c", "a<br/>b<br/>c"),
            // dropped containers keep their text
            ("<div><h1>t</h1><span>s</span></div>", "ts"),
            ("<style>p{}</style>", "p{}"),
            ("", ""),
            ("plain text", "plain text"),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_subset(input), expected, "input: {input}");
        }
    }
}
