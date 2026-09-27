use url::Url;

/// Metadata extracted from a web page's HTML or oEmbed responses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtractedMetadata {
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub domain: String,
    pub reader_content: Option<String>,
    pub reading_time_minutes: i64,
}

/// Extracts the clean host domain from a URL (e.g. `news.ycombinator.com`).
pub fn extract_domain(raw_url: &str) -> String {
    if let Ok(parsed) = Url::parse(raw_url) {
        if let Some(host) = parsed.host_str() {
            return host.trim_start_matches("www.").to_string();
        }
    }
    "unknown".to_string()
}

/// Extracts the YouTube video ID from `youtu.be/<id>` or `youtube.com/watch?v=<id>`.
pub fn extract_youtube_video_id(raw_url: &str) -> Option<String> {
    let parsed = Url::parse(raw_url).ok()?;
    let host = parsed.host_str()?;
    if host.contains("youtu.be") {
        let id = parsed.path().trim_matches('/');
        if !id.is_empty() {
            return Some(id.to_string());
        }
    } else if host.contains("youtube.com") {
        for (k, v) in parsed.query_pairs() {
            if k == "v" && !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Parses HTML content to extract title, description, author, image, and reader content.
pub fn parse_html_metadata(html: &str, meta: &mut ExtractedMetadata) {
    let lower_html = html.to_lowercase();

    // 1. Title: og:title -> twitter:title -> <title>
    if let Some(og_title) = extract_meta_attr(html, "property", "og:title") {
        meta.title = clean_text(&og_title);
    } else if let Some(tw_title) = extract_meta_attr(html, "name", "twitter:title") {
        meta.title = clean_text(&tw_title);
    } else if let Some(tag_title) = extract_tag_content(html, &lower_html, "title") {
        meta.title = clean_text(&tag_title);
    }

    // 2. Description: og:description -> twitter:description -> description
    if let Some(desc) = extract_meta_attr(html, "property", "og:description")
        .or_else(|| extract_meta_attr(html, "name", "twitter:description"))
        .or_else(|| extract_meta_attr(html, "name", "description"))
    {
        let cleaned = clean_text(&desc);
        if !cleaned.is_empty() {
            meta.description = Some(cleaned);
        }
    }

    // 3. Author
    if let Some(author) = extract_meta_attr(html, "name", "author")
        .or_else(|| extract_meta_attr(html, "property", "article:author"))
    {
        let cleaned = clean_text(&author);
        if !cleaned.is_empty() {
            meta.author = Some(cleaned);
        }
    }

    // 4. Image: og:image -> twitter:image
    if let Some(img) = extract_meta_attr(html, "property", "og:image")
        .or_else(|| extract_meta_attr(html, "name", "twitter:image"))
    {
        let trimmed = img.trim();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            meta.image_url = Some(trimmed.to_string());
        }
    }

    // 5. Reader content: extract text from <p> paragraphs
    let paragraphs = extract_paragraphs(html);
    if !paragraphs.is_empty() {
        let content = paragraphs.join("\n\n");
        let word_count = content.split_whitespace().count();
        let minutes = ((word_count as f64) / 200.0).ceil() as i64;
        meta.reading_time_minutes = minutes.max(1);
        meta.reader_content = Some(content);
    }
}

pub fn clean_text(raw: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;

    for ch in raw.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            out.push(ch);
        }
    }

    out.replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_meta_attr(html: &str, attr_name: &str, target_key: &str) -> Option<String> {
    let lower_target = target_key.to_lowercase();
    let lower_attr = attr_name.to_lowercase();

    for tag in html.split("<meta ").skip(1) {
        let tag_end = tag.find('>').unwrap_or(tag.len());
        let meta_tag = &tag[..tag_end];
        let lower_meta = meta_tag.to_lowercase();

        let pattern1 = format!("{lower_attr}=\"{lower_target}\"");
        let pattern2 = format!("{lower_attr}='{lower_target}'");

        if lower_meta.contains(&pattern1) || lower_meta.contains(&pattern2) {
            if let Some(content) = extract_attribute_value(meta_tag, "content") {
                return Some(content);
            }
        }
    }
    None
}

fn extract_attribute_value(tag_slice: &str, attr: &str) -> Option<String> {
    let lower_slice = tag_slice.to_lowercase();
    let needle = format!("{attr}=");
    let pos = lower_slice.find(&needle)?;

    let after_eq = &tag_slice[pos + needle.len()..];
    let quote_char = after_eq.chars().next()?;

    if quote_char == '"' || quote_char == '\'' {
        let rest = &after_eq[1..];
        let end_pos = rest.find(quote_char)?;
        Some(rest[..end_pos].to_string())
    } else {
        let end_pos = after_eq
            .find(|c: char| c.is_whitespace() || c == '/' || c == '>')
            .unwrap_or(after_eq.len());
        Some(after_eq[..end_pos].to_string())
    }
}

fn extract_tag_content(html: &str, lower_html: &str, tag_name: &str) -> Option<String> {
    let open_tag = format!("<{tag_name}");
    let close_tag = format!("</{tag_name}>");

    let open_idx = lower_html.find(&open_tag)?;
    let start_of_content = html[open_idx..].find('>')? + open_idx + 1;
    let close_idx = lower_html[start_of_content..].find(&close_tag)? + start_of_content;

    Some(html[start_of_content..close_idx].to_string())
}

fn extract_paragraphs(html: &str) -> Vec<String> {
    let mut paragraphs = Vec::new();
    let lower = html.to_lowercase();
    let mut cursor = 0;

    while let Some(start_p) = lower[cursor..].find("<p") {
        let abs_start = cursor + start_p;
        let Some(tag_close) = html[abs_start..].find('>') else {
            break;
        };
        let content_start = abs_start + tag_close + 1;

        let Some(end_p) = lower[content_start..].find("</p>") else {
            break;
        };
        let abs_end = content_start + end_p;

        let p_text = clean_text(&html[content_start..abs_end]);
        if p_text.len() > 30 {
            paragraphs.push(p_text);
        }

        cursor = abs_end + 4;
        if paragraphs.len() >= 25 {
            break;
        }
    }

    paragraphs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://www.github.com/rust-lang"),
            "github.com"
        );
        assert_eq!(
            extract_domain("http://news.ycombinator.com/item?id=123"),
            "news.ycombinator.com"
        );
        assert_eq!(extract_domain("not-a-valid-url"), "unknown");
    }

    #[test]
    fn test_parse_html_metadata() {
        let html = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Ignored Tag Title</title>
                <meta property="og:title" content="Rust in 2026: Fast &amp; Reliable" />
                <meta property="og:description" content="A deep dive into asynchronous systems." />
                <meta property="og:image" content="https://example.com/cover.png" />
                <meta name="author" content="Ferris the Crab" />
            </head>
            <body>
                <p>First paragraph about memory safety, ownership, and zero-cost abstractions in software engineering.</p>
                <p>Second paragraph detailing fearless concurrency and building high throughput services.</p>
            </body>
            </html>
        "#;

        let mut meta = ExtractedMetadata::default();
        parse_html_metadata(html, &mut meta);

        assert_eq!(meta.title, "Rust in 2026: Fast & Reliable");
        assert_eq!(
            meta.description.as_deref(),
            Some("A deep dive into asynchronous systems.")
        );
        assert_eq!(meta.author.as_deref(), Some("Ferris the Crab"));
        assert_eq!(
            meta.image_url.as_deref(),
            Some("https://example.com/cover.png")
        );
        assert_eq!(meta.reading_time_minutes, 1);
        assert!(meta.reader_content.unwrap().contains("memory safety"));
    }

    #[test]
    fn test_extract_youtube_video_id() {
        assert_eq!(
            extract_youtube_video_id("https://www.youtube.com/watch?v=i_1f5OEJIAY"),
            Some("i_1f5OEJIAY".to_string())
        );
        assert_eq!(
            extract_youtube_video_id("https://youtu.be/i_1f5OEJIAY"),
            Some("i_1f5OEJIAY".to_string())
        );
        assert_eq!(
            extract_youtube_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_youtube_video_id("https://github.com/rust-lang"),
            None
        );
    }
}
