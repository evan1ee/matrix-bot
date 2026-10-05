use matrix_sdk::room::Room;
use matrix_sdk::ruma::events::room::message::{MessageType, Relation, SyncRoomMessageEvent};
use matrix_sdk::ruma::events::AnySyncMessageLikeEvent;
use matrix_sdk::ruma::events::AnySyncTimelineEvent;
use matrix_sdk::ruma::EventId;
use tracing::warn;

pub use crate::domain::chat::QuotedMessage;

/// Extracts any quoted message context and returns (optional_quote, unquoted_prompt).
pub fn extract_quote_and_prompt(
    body: &str,
    formatted_body: Option<&str>,
) -> (Option<QuotedMessage>, String) {
    // 1. Try formatted HTML first (Matrix rich reply standard <mx-reply>)
    if let Some(html) = formatted_body {
        if let Some((quote, unquoted)) = extract_from_html(html) {
            return (Some(quote), unquoted);
        }
    }

    // 2. Try plain-text quote extraction (> quote fallback)
    if let Some((quote, unquoted)) = extract_from_plain_text(body) {
        return (Some(quote), unquoted);
    }

    (None, body.trim().to_string())
}

/// Extracts replied event ID from room message relation (if present).
pub fn extract_replied_event_id(
    relates_to: &Option<
        Relation<matrix_sdk::ruma::events::room::message::RoomMessageEventContentWithoutRelation>,
    >,
) -> Option<String> {
    match relates_to {
        Some(Relation::Reply(reply)) => Some(reply.in_reply_to.event_id.to_string()),
        Some(Relation::Thread(thread)) => {
            thread.in_reply_to.as_ref().map(|i| i.event_id.to_string())
        }
        _ => None,
    }
}

/// Attempts to resolve a quoted event directly from the room if the body lacked quote text.
pub async fn resolve_quoted_event_from_room(
    room: &Room,
    event_id_str: &str,
) -> Option<QuotedMessage> {
    let eid = EventId::parse(event_id_str).ok()?;
    let timeline_event = room.event(&eid, None).await.ok()?;
    let any_event = match timeline_event.raw().deserialize() {
        Ok(ev) => ev,
        Err(err) => {
            warn!(
                "Failed to deserialize timeline event {}: {:?}",
                event_id_str, err
            );
            return None;
        }
    };

    match any_event {
        AnySyncTimelineEvent::MessageLike(AnySyncMessageLikeEvent::RoomMessage(
            SyncRoomMessageEvent::Original(msg),
        )) => {
            let author = msg.sender.to_string();
            let body = match msg.content.msgtype {
                MessageType::Text(t) => t.body,
                MessageType::Notice(n) => n.body,
                MessageType::Emote(e) => format!("* {} {}", author, e.body),
                _ => return None,
            };
            Some(QuotedMessage {
                author: Some(author),
                body: body.trim().to_string(),
                event_id: Some(event_id_str.to_string()),
            })
        }
        _ => None,
    }
}

/// Extracts quote from Matrix rich reply HTML (<mx-reply><blockquote>...</blockquote></mx-reply>).
fn extract_from_html(html: &str) -> Option<(QuotedMessage, String)> {
    let start_tag = "<mx-reply>";
    let end_tag = "</mx-reply>";

    let start_idx = html.find(start_tag)?;
    let end_idx = html.find(end_tag)?;
    if end_idx <= start_idx {
        return None;
    }

    let reply_block = &html[start_idx + start_tag.len()..end_idx];
    let after_reply = &html[end_idx + end_tag.len()..];

    // Extract author if present inside <a href=".../@user:server"> or <a href="...">@user:server</a>
    let author = extract_author_from_html(reply_block);

    // Clean up quote text: strip In reply to links, blockquote tags, and HTML tags
    let mut cleaned_html = reply_block.to_string();
    if let Some(pos) = cleaned_html.find("In reply to") {
        if let Some(br_pos) = cleaned_html[pos..].find("<br") {
            cleaned_html = cleaned_html[pos + br_pos..].to_string();
        }
    }
    cleaned_html = cleaned_html
        .replace("<blockquote>", "")
        .replace("</blockquote>", "");

    let quote_text = crate::utils::strip_html(&cleaned_html).trim().to_string();
    if quote_text.is_empty() {
        return None;
    }

    let unquoted = crate::utils::strip_html(after_reply).trim().to_string();

    Some((
        QuotedMessage {
            author,
            body: quote_text,
            event_id: None,
        },
        unquoted,
    ))
}

/// Extracts author from HTML reply block.
fn extract_author_from_html(html: &str) -> Option<String> {
    for segment in html.split('<') {
        if let Some(gt_idx) = segment.find('>') {
            let inner = &segment[gt_idx + 1..];
            let trimmed = inner.trim();
            if trimmed.starts_with('@') && trimmed.contains(':') {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extracts quote from plain text lines starting with '>'.
fn extract_from_plain_text(body: &str) -> Option<(QuotedMessage, String)> {
    let lines: Vec<&str> = body.lines().collect();
    if lines.is_empty() {
        return None;
    }

    // Check if lines start with '>'
    let mut quote_lines = Vec::new();
    let mut prompt_lines = Vec::new();
    let mut in_quote = false;
    let mut has_quote = false;

    for line in lines {
        let trimmed = line.trim_start();
        if trimmed.starts_with('>') {
            in_quote = true;
            has_quote = true;
            let stripped = trimmed.trim_start_matches('>').trim_start();
            quote_lines.push(stripped);
        } else {
            if in_quote {
                in_quote = false;
            }
            prompt_lines.push(line);
        }
    }

    if !has_quote || quote_lines.iter().all(|l| l.trim().is_empty()) {
        return None;
    }

    // Check first quote line for author: <@author:server> or @author:
    let first_line = quote_lines[0].trim();
    let (author, first_line_body) = extract_author_from_plain(first_line);
    if let Some(flb) = first_line_body {
        quote_lines[0] = flb;
    }

    let quote_body = quote_lines.join("\n").trim().to_string();
    if quote_body.is_empty() {
        return None;
    }

    let prompt = prompt_lines.join("\n").trim().to_string();

    Some((
        QuotedMessage {
            author,
            body: quote_body,
            event_id: None,
        },
        prompt,
    ))
}

/// Extracts author from the first plain text quote line like `<@alice:example.com> Rest of text`.
fn extract_author_from_plain(first_line: &str) -> (Option<String>, Option<&str>) {
    if first_line.starts_with('<') {
        if let Some(close_idx) = first_line.find('>') {
            let candidate = &first_line[1..close_idx];
            if candidate.starts_with('@') {
                let rest = first_line[close_idx + 1..].trim();
                return (Some(candidate.to_string()), Some(rest));
            }
        }
    } else if first_line.starts_with('@') {
        if let Some(space_idx) = first_line.find(' ') {
            let candidate = first_line[..space_idx].trim_end_matches(':');
            if candidate.contains(':') {
                let rest = first_line[space_idx + 1..].trim();
                return (Some(candidate.to_string()), Some(rest));
            }
        }
    }

    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_from_plain_text_standard_reply() {
        let msg = "> <@alice:matrix.org> Global temperatures rose by 1.5 degrees.\n> More info soon.\n\n@bot fact check this";
        let (quote_opt, prompt) = extract_quote_and_prompt(msg, None);
        assert!(quote_opt.is_some());
        let q = quote_opt.unwrap();
        assert_eq!(q.author.as_deref(), Some("@alice:matrix.org"));
        assert_eq!(
            q.body,
            "Global temperatures rose by 1.5 degrees.\nMore info soon."
        );
        assert_eq!(prompt, "@bot fact check this");
    }

    #[test]
    fn test_extract_from_plain_text_manual_quote() {
        let msg = "> Salt water cures aging if drunk daily.\n\n@bot is this true?";
        let (quote_opt, prompt) = extract_quote_and_prompt(msg, None);
        assert!(quote_opt.is_some());
        let q = quote_opt.unwrap();
        assert_eq!(q.author, None);
        assert_eq!(q.body, "Salt water cures aging if drunk daily.");
        assert_eq!(prompt, "@bot is this true?");
    }

    #[test]
    fn test_extract_from_html_matrix_reply() {
        let html = "<mx-reply><blockquote><a href=\"https://matrix.to/#/!room/$event\">In reply to</a> <a href=\"https://matrix.to/#/@bob:matrix.org\">@bob:matrix.org</a><br>Please submit your expenses by Friday 5pm.</blockquote></mx-reply><a href=\"https://matrix.to/#/@bot:matrix.org\">@bot</a> add todo";
        let plain =
            "> <@bob:matrix.org> Please submit your expenses by Friday 5pm.\n\n@bot add todo";
        let (quote_opt, prompt) = extract_quote_and_prompt(plain, Some(html));
        assert!(quote_opt.is_some());
        let q = quote_opt.unwrap();
        assert_eq!(q.author.as_deref(), Some("@bob:matrix.org"));
        assert_eq!(q.body, "Please submit your expenses by Friday 5pm.");
        assert_eq!(prompt, "@bot add todo");
    }

    #[test]
    fn test_no_quote() {
        let msg = "!todo add Buy milk";
        let (quote_opt, prompt) = extract_quote_and_prompt(msg, None);
        assert!(quote_opt.is_none());
        assert_eq!(prompt, "!todo add Buy milk");
    }

    #[test]
    fn test_to_context_prompt_formatting() {
        let q = QuotedMessage {
            author: Some("@alice:matrix.org".to_string()),
            body: "The earth is flat.".to_string(),
            event_id: None,
        };
        let ctx_prompt = q.to_context_prompt("fact check this");
        assert!(ctx_prompt.starts_with("<quoted_message author=\"@alice:matrix.org\">"));
        assert!(ctx_prompt.contains("</quoted_message>"));
        assert!(ctx_prompt.contains("The earth is flat."));
        assert!(ctx_prompt.contains("User request: fact check this"));
    }
}
