//! Tag and recurrence handling for the todo chat commands: the pure text
//! parsers used by `!todo add`, `!todo list|room`, and the `tag`, `tags` and
//! `repeat` handlers. Rules live in the domain and `TodoService`; this file
//! only reads chat text and renders replies.

use std::time::Duration;

use chrono_tz::Tz;

use crate::application::todos::TodoService;
use crate::application::AppError;
use crate::domain::accounts::AccountId;
use crate::domain::reminders::RecurrencePattern;
use crate::domain::tags::Tag;
use crate::domain::todos::{TodoItem, TodoPatch};
use crate::interface::chat::error_reply;
use crate::utils::{format_datetime, html_escape, parse_reminder_args};

pub const NEEDS_DUE: &str = "⚠️ A repeating task needs a due date, e.g. \
     <code>!todo add Pay rent every month due 3d</code>";
const TAG_USAGE: &str = "⚠️ Usage: <code>!todo tag &lt;id&gt; +work -home</code>";
const REPEAT_USAGE: &str = "⚠️ Usage: <code>!todo repeat &lt;id&gt; \
     &lt;daily|weekly|monthly|yearly|none&gt;</code>";

/// What `!todo add` reads out of the free text after the priority.
#[derive(Debug, PartialEq, Eq)]
pub struct AddExtras {
    pub text: String,
    pub tags: Vec<Tag>,
    pub recurrence: RecurrencePattern,
    pub due: Option<Duration>,
}

impl AddExtras {
    /// Text taken as written: nothing extracted, no tags, no repeat, no due.
    pub fn plain(text: &str) -> Self {
        Self {
            text: text.to_string(),
            tags: Vec::new(),
            recurrence: RecurrencePattern::None,
            due: None,
        }
    }
}

/// Arguments of `!todo list|room`: a 1-based page and an optional tag filter.
#[derive(Debug, PartialEq, Eq)]
pub struct ListArgs {
    pub page: i64,
    pub tag: Option<Tag>,
}

/// Removes every whole `#token` that is a valid tag. `C#` and `#a.b` are not
/// tags, so they stay in the text. Whitespace collapses to single spaces.
pub fn extract_tags(text: &str) -> (String, Vec<Tag>) {
    let mut tags = Vec::new();
    let mut kept = Vec::new();
    for token in text.split_whitespace() {
        match token.strip_prefix('#').map(|_| Tag::parse(token)) {
            Some(Ok(tag)) => tags.push(tag),
            _ => kept.push(token),
        }
    }
    tags.sort();
    tags.dedup();
    (kept.join(" "), tags)
}

pub fn parse_recurrence_word(s: &str) -> Option<RecurrencePattern> {
    match s.to_lowercase().as_str() {
        "day" | "daily" => Some(RecurrencePattern::Daily),
        "week" | "weekly" => Some(RecurrencePattern::Weekly),
        "month" | "monthly" => Some(RecurrencePattern::Monthly),
        "year" | "yearly" => Some(RecurrencePattern::Yearly),
        "none" => Some(RecurrencePattern::None),
        _ => None,
    }
}

/// Removes the first `every <day|week|month|year>` pair. `every none` and
/// `every other day` are ordinary prose and stay.
pub fn extract_every(text: &str) -> (String, Option<RecurrencePattern>) {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let found = tokens.windows(2).position(|pair| {
        pair[0].eq_ignore_ascii_case("every")
            && matches!(
                parse_recurrence_word(pair[1]),
                Some(p) if p != RecurrencePattern::None
            )
    });
    match found {
        Some(i) => {
            let pattern = parse_recurrence_word(tokens[i + 1]);
            let kept: Vec<&str> = tokens
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i && *j != i + 1)
                .map(|(_, t)| *t)
                .collect();
            (kept.join(" "), pattern)
        }
        None => (tokens.join(" "), None),
    }
}

/// Splits a trailing ` due <duration>` off `text`. Only a single relative
/// token such as `3d` counts, so "due to the rain" stays part of the task.
fn split_due(text: &str) -> (String, Option<Duration>) {
    let lower = text.to_ascii_lowercase();
    if let Some(idx) = lower.rfind(" due ") {
        let arg = text[idx + 5..].trim();
        if !arg.contains(char::is_whitespace) {
            if let Some((duration, _)) = parse_reminder_args(arg) {
                return (text[..idx].trim().to_string(), Some(duration));
            }
        }
    }
    (text.to_string(), None)
}

/// Reads `every <word>`, `#tags` and a trailing `due <duration>` out of the
/// text of `!todo add`. A recurrence without a due date is refused here with
/// the reply to show; the service refuses it again for every other caller.
pub fn split_add_extras(text: &str) -> Result<AddExtras, String> {
    let (text, every) = extract_every(text);
    let (text, tags) = extract_tags(&text);
    let (text, due) = split_due(&text);
    let recurrence = every.unwrap_or_default();
    if recurrence != RecurrencePattern::None && due.is_none() {
        return Err(NEEDS_DUE.to_string());
    }
    Ok(AddExtras {
        text,
        tags,
        recurrence,
        due,
    })
}

fn invalid_tag(err: impl std::fmt::Display) -> String {
    format!("⚠️ {}", html_escape(&err.to_string()))
}

/// `+work -home` (a bare `work` adds) into `(add, remove)`.
pub fn parse_tag_edits(args: &str) -> Result<(Vec<Tag>, Vec<Tag>), String> {
    let (mut add, mut remove) = (Vec::new(), Vec::new());
    for token in args.split_whitespace() {
        if let Some(name) = token.strip_prefix('-') {
            remove.push(Tag::parse(name).map_err(invalid_tag)?);
        } else {
            let name = token.strip_prefix('+').unwrap_or(token);
            add.push(Tag::parse(name).map_err(invalid_tag)?);
        }
    }
    if add.is_empty() && remove.is_empty() {
        return Err(TAG_USAGE.to_string());
    }
    Ok((add, remove))
}

/// Page number and `#tag` after `list` or `room`, in any order. Other words
/// are ignored, as the unfiltered views always did.
pub fn parse_list_args(args_body: &str) -> Result<ListArgs, String> {
    let mut parsed = ListArgs { page: 1, tag: None };
    for token in args_body.split_whitespace().skip(1) {
        if token.starts_with('#') {
            parsed.tag = Some(Tag::parse(token).map_err(invalid_tag)?);
        } else if let Ok(page) = token.parse::<i64>() {
            parsed.page = page.max(1);
        }
    }
    Ok(parsed)
}

/// The reply for a failed service call. A missing todo is named by id; every
/// other variant goes through the shared `error_reply`, which keeps database
/// and dependency detail out of the room.
pub fn failure(err: AppError, id: i64) -> String {
    match err {
        AppError::NotFound(msg) if msg.contains("step") => {
            format!("⚠️ Task <b>#{id}</b> has no such step.")
        }
        AppError::NotFound(_) => format!("⚠️ Task <b>#{id}</b> not found."),
        other => error_reply(other).html_body().to_owned(),
    }
}

/// `<code>#a #b</code>`, or nothing for an untagged todo.
pub fn tags_code(tags: &[Tag]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let joined: Vec<String> = tags.iter().map(|t| format!("#{}", t.as_str())).collect();
    format!("<code>{}</code>", html_escape(&joined.join(" ")))
}

/// What `!todo add` reports beyond the task itself: due date, tags, repeat.
pub fn added_details(item: &TodoItem, tz: &Tz) -> String {
    let mut parts = Vec::new();
    if let Some(due) = &item.due_at {
        parts.push(format!("📅 {}", html_escape(&format_datetime(due, tz))));
    }
    if item.recurrence != RecurrencePattern::None {
        parts.push(format!("↻ {}", item.recurrence.as_str()));
    }
    if !item.tags.is_empty() {
        parts.push(tags_code(&item.tags));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("<br/>{}", parts.join(" "))
    }
}

pub async fn handle_tags(todos: &TodoService, owner_id: &AccountId) -> Option<String> {
    match todos.list_tags(owner_id).await {
        Ok(tags) if tags.is_empty() => Some(
            "🏷️ No tags yet. Add one with <code>!todo tag &lt;id&gt; +work</code>.".to_string(),
        ),
        Ok(tags) => {
            let mut html = "🏷️ <b>Tags</b> (active tasks):<br/>".to_string();
            for (tag, count) in tags {
                html.push_str(&format!("• {} — {}<br/>", tags_code(&[tag]), count));
            }
            Some(html)
        }
        Err(err) => Some(error_reply(err).html_body().to_owned()),
    }
}

pub async fn handle_tag(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
) -> Option<String> {
    let rest = args_body[3..].trim();
    let (id_str, edits) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let Ok(id) = id_str.parse::<i64>() else {
        return Some(TAG_USAGE.to_string());
    };
    let (add, remove) = match parse_tag_edits(edits) {
        Ok(parsed) => parsed,
        Err(reply) => return Some(reply),
    };

    match todos.edit_tags(owner_id, id, &add, &remove).await {
        Ok(item) if item.tags.is_empty() => {
            Some(format!("🏷️ Task <b>#{}</b> has no tags.", item.id))
        }
        Ok(item) => Some(format!(
            "🏷️ Task <b>#{}</b> tags: {}",
            item.id,
            tags_code(&item.tags)
        )),
        Err(err) => Some(failure(err, id)),
    }
}

pub async fn handle_repeat(
    todos: &TodoService,
    owner_id: &AccountId,
    args_body: &str,
) -> Option<String> {
    let mut words = args_body[6..].split_whitespace();
    let id = words.next().and_then(|s| s.parse::<i64>().ok());
    let pattern = words.next().and_then(parse_recurrence_word);
    let (Some(id), Some(recurrence), None) = (id, pattern, words.next()) else {
        return Some(REPEAT_USAGE.to_string());
    };
    let patch = TodoPatch {
        recurrence: Some(recurrence),
        ..Default::default()
    };
    match todos.update(owner_id, id, patch).await {
        Ok((item, _)) if item.recurrence == RecurrencePattern::None => {
            Some(format!("↻ Task <b>#{}</b> no longer repeats.", item.id))
        }
        Ok((item, _)) => Some(format!(
            "↻ Task <b>#{}</b> now repeats {}.",
            item.id,
            item.recurrence.as_str()
        )),
        Err(err) => Some(failure(err, id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> Tag {
        Tag::parse(s).unwrap()
    }

    #[test]
    fn extracts_tags_and_keeps_remaining_text() {
        let (text, tags) = extract_tags("Fix bug #123 in C# code #Work");
        assert_eq!(text, "Fix bug in C# code");
        assert_eq!(tags, vec![tag("123"), tag("work")]);
    }

    #[test]
    fn extract_tags_dedupes_and_ignores_unparsable_hashes() {
        let (text, tags) = extract_tags("a #x #X # #a.b b");
        assert_eq!(text, "a # #a.b b");
        assert_eq!(tags, vec![tag("x")]);
    }

    #[test]
    fn extract_every_parses_inline() {
        assert_eq!(
            extract_every("Pay rent every month"),
            ("Pay rent".to_string(), Some(RecurrencePattern::Monthly))
        );
        assert_eq!(
            extract_every("Every Day stretch"),
            ("stretch".to_string(), Some(RecurrencePattern::Daily))
        );
        assert_eq!(
            extract_every("everything ok"),
            ("everything ok".to_string(), None)
        );
        assert_eq!(
            extract_every("do it every other day"),
            ("do it every other day".to_string(), None)
        );
    }

    #[test]
    fn recurrence_words() {
        for (w, p) in [
            ("day", RecurrencePattern::Daily),
            ("Daily", RecurrencePattern::Daily),
            ("week", RecurrencePattern::Weekly),
            ("weekly", RecurrencePattern::Weekly),
            ("month", RecurrencePattern::Monthly),
            ("monthly", RecurrencePattern::Monthly),
            ("year", RecurrencePattern::Yearly),
            ("yearly", RecurrencePattern::Yearly),
            ("none", RecurrencePattern::None),
        ] {
            assert_eq!(parse_recurrence_word(w), Some(p), "{w}");
        }
        assert_eq!(parse_recurrence_word("hourly"), None);
    }

    #[test]
    fn tag_edit_args_parse() {
        let (add, remove) = parse_tag_edits("+work -home").unwrap();
        assert_eq!((add, remove), (vec![tag("work")], vec![tag("home")]));
        let (add, remove) = parse_tag_edits("work #play").unwrap();
        assert_eq!((add, remove), (vec![tag("work"), tag("play")], vec![]));
        assert!(parse_tag_edits("").is_err());
        assert!(parse_tag_edits("+bad.tag").is_err());
        assert!(parse_tag_edits("+").is_err());
    }

    #[test]
    fn add_extras_combine_every_tags_and_due() {
        let e = split_add_extras("Pay rent #home every month due 3d").unwrap();
        assert_eq!(e.text, "Pay rent");
        assert_eq!(e.tags, vec![tag("home")]);
        assert_eq!(e.recurrence, RecurrencePattern::Monthly);
        assert_eq!(e.due, Some(Duration::from_secs(3 * 86400)));
    }

    #[test]
    fn add_extras_leave_a_prose_due_alone() {
        let e = split_add_extras("File taxes due to the deadline").unwrap();
        assert_eq!(e.text, "File taxes due to the deadline");
        assert_eq!(e.due, None);
    }

    #[test]
    fn recurrence_without_due_is_refused() {
        assert_eq!(
            split_add_extras("Pay rent every month"),
            Err(NEEDS_DUE.to_string())
        );
    }

    #[test]
    fn list_args_take_page_and_tag_in_any_order() {
        assert_eq!(
            parse_list_args("list").unwrap(),
            ListArgs { page: 1, tag: None }
        );
        assert_eq!(
            parse_list_args("list 3").unwrap(),
            ListArgs { page: 3, tag: None }
        );
        assert_eq!(
            parse_list_args("room #Work 2").unwrap(),
            ListArgs {
                page: 2,
                tag: Some(tag("work"))
            }
        );
        assert!(parse_list_args("list #bad.tag").is_err());
    }
}
