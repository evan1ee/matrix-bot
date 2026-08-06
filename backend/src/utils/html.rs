/// Escapes HTML special characters for safe inclusion in Matrix HTML messages
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Strips HTML tags for clean plain-text notification fallbacks
pub fn strip_html(html: &str) -> String {
    let preprocessed = html
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("</p>", "\n");

    let mut result = String::with_capacity(preprocessed.len());
    let mut inside_tag = false;

    for c in preprocessed.chars() {
        match c {
            '<' => inside_tag = true,
            '>' => inside_tag = false,
            _ if !inside_tag => result.push(c),
            _ => {}
        }
    }

    result
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

/// Formats a badge for priority or status
pub fn priority_badge(priority: &str) -> String {
    match priority.to_lowercase().as_str() {
        "high" => "🔴 <b>[HIGH]</b>",
        "medium" | "med" => "🟡 <b>[MED]</b>",
        "low" => "🔵 <b>[LOW]</b>",
        _ => "",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_html_escape() {
        assert_eq!(
            html_escape("<script>alert('XSS & \"fun\"');</script>"),
            "&lt;script&gt;alert(&#39;XSS &amp; &quot;fun&quot;&#39;);&lt;/script&gt;"
        );
        assert_eq!(html_escape("Hello World"), "Hello World");
    }

    #[test]
    fn test_strip_html() {
        assert_eq!(
            strip_html("⏰ <b>REMINDER for Alice!</b><br/>Check oven"),
            "⏰ REMINDER for Alice!\nCheck oven"
        );
        assert_eq!(strip_html("Hello <i>World</i>"), "Hello World");
    }

    #[test]
    fn test_priority_badge() {
        assert_eq!(priority_badge("high"), "🔴 <b>[HIGH]</b>");
        assert_eq!(priority_badge("med"), "🟡 <b>[MED]</b>");
        assert_eq!(priority_badge("low"), "🔵 <b>[LOW]</b>");
        assert_eq!(priority_badge("unknown"), "");
    }
}
