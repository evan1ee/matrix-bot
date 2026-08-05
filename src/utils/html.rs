/// Escapes HTML special characters for safe inclusion in Matrix HTML messages
pub fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
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
        assert_eq!(html_escape("<script>alert(\"XSS & fun\");</script>"), "&lt;script&gt;alert(&quot;XSS &amp; fun&quot;);&lt;/script&gt;");
        assert_eq!(html_escape("Hello World"), "Hello World");
    }

    #[test]
    fn test_priority_badge() {
        assert_eq!(priority_badge("high"), "🔴 <b>[HIGH]</b>");
        assert_eq!(priority_badge("med"), "🟡 <b>[MED]</b>");
        assert_eq!(priority_badge("low"), "🔵 <b>[LOW]</b>");
        assert_eq!(priority_badge("unknown"), "");
    }
}
