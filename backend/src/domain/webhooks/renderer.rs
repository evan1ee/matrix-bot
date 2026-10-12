//! Webhook alert message rendering, markdown parsing, and HTML sanitization.

use crate::domain::webhooks::WebhookSeverity;
use crate::utils::html::{html_escape, strip_html};

/// Converts basic Markdown styling into safe Matrix-compatible HTML.
pub fn markdown_to_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    let mut in_code_block = false;

    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            if in_code_block {
                out.push_str("</code></pre>");
                in_code_block = false;
            } else {
                let _lang = trimmed.trim_start_matches('`').trim();
                out.push_str("<pre><code>");
                in_code_block = true;
            }
            continue;
        }

        if in_code_block {
            out.push_str(&html_escape(line));
            out.push('\n');
            continue;
        }

        let escaped_line = html_escape(line);
        let formatted = format_inline_markdown(&escaped_line);

        if !out.is_empty() && !out.ends_with("<pre><code>") {
            out.push_str("<br/>");
        }
        out.push_str(&formatted);
    }

    if in_code_block {
        out.push_str("</code></pre>");
    }

    out
}

/// Applies bold `**bold**`, inline code `` `code` ``, and italics `*italic*` to escaped text.
fn format_inline_markdown(escaped: &str) -> String {
    let mut res = String::with_capacity(escaped.len() + 16);
    let mut chars = escaped.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '`' {
            let mut code = String::new();
            let mut closed = false;
            for next_c in chars.by_ref() {
                if next_c == '`' {
                    closed = true;
                    break;
                }
                code.push(next_c);
            }
            if closed {
                res.push_str("<code>");
                res.push_str(&code);
                res.push_str("</code>");
            } else {
                res.push('`');
                res.push_str(&code);
            }
        } else if c == '*' && chars.peek() == Some(&'*') {
            chars.next(); // consume second '*'
            let mut bold = String::new();
            let mut closed = false;
            while let Some(next_c) = chars.next() {
                if next_c == '*' && chars.peek() == Some(&'*') {
                    chars.next();
                    closed = true;
                    break;
                }
                bold.push(next_c);
            }
            if closed {
                res.push_str("<strong>");
                res.push_str(&bold);
                res.push_str("</strong>");
            } else {
                res.push_str("**");
                res.push_str(&bold);
            }
        } else {
            res.push(c);
        }
    }

    res
}

/// Decodes HTML numeric and named entities (`&#106;`, `&#x6a;`, `&colon;`, etc.) so obfuscated
/// `javascript:` / `data:` / `vbscript:` URI schemes cannot evade inspection.
fn decode_html_entities_for_check(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after_amp = &rest[amp + 1..];
        let end_pos = after_amp
            .find(|c: char| c == ';' || !c.is_ascii_alphanumeric() && c != '#')
            .unwrap_or(after_amp.len());
        let entity = &after_amp[..end_pos];
        let consumed = if after_amp[end_pos..].starts_with(';') {
            end_pos + 1
        } else {
            end_pos
        };
        let decoded = if let Some(hex) = entity
            .strip_prefix("#x")
            .or_else(|| entity.strip_prefix("#X"))
        {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = entity.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match entity.to_ascii_lowercase().as_str() {
                "colon" => Some(':'),
                "tab" => Some('\t'),
                "newline" => Some('\n'),
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                _ => None,
            }
        };
        if let Some(ch) = decoded {
            out.push(ch);
            rest = &after_amp[consumed..];
        } else {
            out.push('&');
            rest = after_amp;
        }
    }
    out.push_str(rest);
    out
}

/// Sanitizes an HTML snippet for safe Matrix rendering by allowing only safe tags and attributes,
/// escaping or stripping dangerous elements like <script>, <iframe>, event handlers, and javascript: links.
pub fn sanitize_html(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '<' {
            let mut tag = String::new();
            let mut in_quote: Option<char> = None;
            let mut closed = false;
            while let Some(&next_c) = chars.peek() {
                if let Some(q) = in_quote {
                    if next_c == q {
                        in_quote = None;
                    }
                    tag.push(chars.next().unwrap());
                } else if next_c == '"' || next_c == '\'' {
                    in_quote = Some(next_c);
                    tag.push(chars.next().unwrap());
                } else if next_c == '>' {
                    chars.next();
                    closed = true;
                    break;
                } else {
                    tag.push(chars.next().unwrap());
                }
            }

            let trimmed_tag = tag.trim();
            if !closed {
                out.push_str("&lt;");
                out.push_str(&html_escape(trimmed_tag));
                continue;
            }
            let clean_tag = trimmed_tag.trim_start_matches('/').trim();
            let tag_name = clean_tag
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase();

            let is_allowed = matches!(
                tag_name.as_str(),
                "b" | "strong"
                    | "i"
                    | "em"
                    | "u"
                    | "del"
                    | "s"
                    | "code"
                    | "pre"
                    | "blockquote"
                    | "br"
                    | "p"
                    | "ul"
                    | "ol"
                    | "li"
                    | "font"
                    | "span"
                    | "a"
                    | "hr"
            );

            let lower_tag = trimmed_tag.to_lowercase();
            let decoded_tag = decode_html_entities_for_check(&lower_tag).to_lowercase();
            let stripped_whitespace: String = decoded_tag
                .chars()
                .filter(|c| !c.is_whitespace() && !c.is_control())
                .collect();
            let has_dangerous_attr = stripped_whitespace.contains("javascript:")
                || stripped_whitespace.contains("data:")
                || stripped_whitespace.contains("vbscript:")
                || decoded_tag.contains("autofocus")
                || decoded_tag.contains("formaction")
                || decoded_tag
                    .split(|c: char| {
                        c.is_whitespace()
                            || c.is_control()
                            || c == '/'
                            || c == '>'
                            || c == '<'
                            || c == '"'
                            || c == '\''
                            || c == '='
                    })
                    .any(|part| {
                        let token = part.trim_start();
                        token.starts_with("on")
                            && token.len() > 2
                            && token
                                .chars()
                                .nth(2)
                                .map(|c| c.is_ascii_alphabetic())
                                .unwrap_or(false)
                    });

            if is_allowed && !has_dangerous_attr {
                out.push('<');
                out.push_str(trimmed_tag);
                out.push('>');
            } else {
                out.push_str("&lt;");
                out.push_str(&html_escape(trimmed_tag));
                out.push_str("&gt;");
            }
        } else {
            out.push(c);
        }
    }

    out
}

/// Builds full Matrix HTML and plain text for an incoming webhook alert.
pub fn render_alert_message(
    webhook_name: &str,
    title: Option<&str>,
    raw_message: &str,
    severity: WebhookSeverity,
    format: Option<&str>,
) -> (String, String) {
    let body_html = match format.map(|f| f.to_lowercase()).as_deref() {
        Some("html") => sanitize_html(raw_message),
        Some("plain") => html_escape(raw_message).replace('\n', "<br/>"),
        _ => markdown_to_html(raw_message),
    };

    let title_escaped = title.map(html_escape);
    let name_escaped = html_escape(webhook_name);
    let emoji = severity.emoji();
    let color = severity.color_hex();
    let sev_label = severity.as_str().to_uppercase();

    let mut html = String::with_capacity(body_html.len() + 256);
    html.push_str("<blockquote>");

    // Header badge
    html.push_str(&format!(
        r#"<font color="{color}"><strong>{emoji} [{sev_label}]</strong></font>"#
    ));

    if let Some(t) = title_escaped {
        html.push_str(&format!(" &mdash; <strong>{t}</strong>"));
    }
    html.push_str("<br/>");

    // Message Body
    html.push_str(&body_html);
    html.push_str("<br/>");

    // Footer
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    html.push_str(&format!(
        r##"<font color="#8b8d98"><small>via <code>{name_escaped}</code> &bull; {now}</small></font>"##
    ));
    html.push_str("</blockquote>");

    let plain = strip_html(&html);
    (html, plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_markdown_to_html_formatting() {
        let md = "Server is **DOWN**\nDetails: `error 500`\n```\npanic: test\n```";
        let html = markdown_to_html(md);
        assert!(html.contains("<strong>DOWN</strong>"));
        assert!(html.contains("<code>error 500</code>"));
        assert!(html.contains("<pre><code>panic: test\n</code></pre>"));
    }

    #[test]
    fn test_render_alert_message() {
        let (html, plain) = render_alert_message(
            "VPS Monitor",
            Some("Host Failure"),
            "Host **alpha** unreachable",
            WebhookSeverity::Critical,
            Some("markdown"),
        );
        assert!(html.contains("🔴 [CRITICAL]"));
        assert!(html.contains("Host Failure"));
        assert!(html.contains("<strong>alpha</strong>"));
        assert!(html.contains("VPS Monitor"));
        assert!(plain.contains("🔴 [CRITICAL]"));
        assert!(plain.contains("Host Failure"));
    }

    #[test]
    fn test_sanitize_html() {
        let input = "<b>Safe Bold</b> <script>alert('xss')</script> <a href=\"javascript:evil()\">click</a> <img src=\"x\" onerror=\"steal()\">";
        let sanitized = sanitize_html(input);
        assert!(sanitized.contains("<b>Safe Bold</b>"));
        assert!(!sanitized.contains("<script>"));
        assert!(sanitized.contains("&lt;script&gt;"));
        assert!(!sanitized.contains("<a href=\"javascript:evil()\">"));
        assert!(sanitized.contains("&lt;a href=&quot;javascript:evil()&quot;&gt;"));
        assert!(!sanitized.contains("<img"));
        assert!(sanitized.contains("&lt;img"));

        let bypass_input = "<a href=\"#\" autofocus onfocus=\"alert(1)\">Click</a> <font onanimationstart=\"steal()\">text</font> <span ontoggle=\"leak()\">badge</span>";
        let bypass_sanitized = sanitize_html(bypass_input);
        assert!(!bypass_sanitized.contains("<a href=\"#\" autofocus onfocus=\"alert(1)\">"));
        assert!(bypass_sanitized
            .contains("&lt;a href=&quot;#&quot; autofocus onfocus=&quot;alert(1)&quot;&gt;"));
        assert!(!bypass_sanitized.contains("<font onanimationstart"));
        assert!(!bypass_sanitized.contains("<span ontoggle"));

        // Unspaced and delimiter-adjacent attribute bypasses
        let unspaced_double = "<a href=\"test\"onclick=\"alert(1)\">Click</a>";
        assert_eq!(
            sanitize_html(unspaced_double),
            "&lt;a href=&quot;test&quot;onclick=&quot;alert(1)&quot;&gt;Click</a>"
        );

        let unspaced_single = "<a href='test'onfocus='alert(1)'>Click</a>";
        assert_eq!(
            sanitize_html(unspaced_single),
            "&lt;a href=&#39;test&#39;onfocus=&#39;alert(1)&#39;&gt;Click</a>"
        );

        let equal_adjacent = "<a href=\"test\"=onclick=\"alert(1)\">Click</a>";
        assert_eq!(
            sanitize_html(equal_adjacent),
            "&lt;a href=&quot;test&quot;=onclick=&quot;alert(1)&quot;&gt;Click</a>"
        );

        let legitimate = "<a href=\"https://example.com\" target=\"_blank\">Link</a> <b>bold</b>";
        assert_eq!(
            sanitize_html(legitimate),
            "<a href=\"https://example.com\" target=\"_blank\">Link</a> <b>bold</b>"
        );

        let quoted_gt = "<a title=\">\" onclick=\"alert(1)\">x</a>";
        assert!(!sanitize_html(quoted_gt).contains("onclick=\"alert(1)\""));

        let entity_js = "<a href=\"&#106;avascript:alert(1)\">x</a> <a href=\"javascript&colon;alert(1)\">y</a>";
        let sanitized_entity_js = sanitize_html(entity_js);
        assert!(!sanitized_entity_js.contains("<a "));
    }
}
