//! Neutral content -> planned Telegram messages and inline keyboard. Pure: nothing
//! here touches the network, so the whole policy (T7, T8, T16) is unit-tested.

use tracing::debug;

use crate::interface::channels::adapter::ChannelError;
use crate::interface::channels::message::{Attachment, AttachmentKind, Content, Visibility};
use crate::interface::channels::render::telegram_html::{html_to_telegram, split_messages};
use crate::utils::html_escape;

/// What a `Private` reply says instead of itself in a group (invariant 10, T8).
pub const PRIVATE_IN_GROUP: &str = "That answer is private — message me directly.";
/// The Bot API upload limit (T16).
pub const MAX_UPLOAD_BYTES: usize = 50 * 1024 * 1024;
/// Buttons on a reply's keyboard (T7).
pub const MAX_BUTTONS: usize = 10;
/// Telegram's `callback_data` limit, in BYTES (T7).
pub const CALLBACK_DATA_MAX: usize = 64;
/// Telegram's button text limit, in characters (T7).
const LABEL_MAX: usize = 64;

#[derive(Debug, PartialEq)]
pub enum Part {
    /// Telegram HTML parse mode.
    Text(String),
    File(Attachment),
}

#[derive(Debug, PartialEq)]
pub struct Plan {
    pub parts: Vec<Part>,
    /// `(label, callback data)`, attached to the last text part.
    pub keyboard: Vec<(String, String)>,
}

impl Plan {
    /// One plain-text message (escaped for HTML mode): the adapter's own notices.
    pub fn text(text: impl AsRef<str>) -> Self {
        Self {
            parts: vec![Part::Text(html_escape(text.as_ref()))],
            keyboard: Vec::new(),
        }
    }
}

/// Which Bot API method uploads a file (T16).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FileMethod {
    Voice,
    Audio,
    Photo,
    Document,
}

pub fn file_method(file: &Attachment) -> FileMethod {
    match file.kind {
        // A voice note is OGG/Opus; the mime may carry parameters (`; codecs=opus`).
        AttachmentKind::Audio if file.mime.starts_with("audio/ogg") => FileMethod::Voice,
        AttachmentKind::Audio => FileMethod::Audio,
        AttachmentKind::Image => FileMethod::Photo,
        AttachmentKind::File => FileMethod::Document,
    }
}

/// T8: a `Private` reply in a group is only the notice. Otherwise the text parts
/// (HTML, or the escaped plain text, split under Telegram's limit), then the
/// files; a file over the upload cap becomes a text notice instead (T16), and
/// the keyboard comes from the actions (T7). Nothing to send at all is
/// `Rejected`: Telegram refuses an empty message.
pub fn plan(
    content: &Content,
    visibility: Visibility,
    in_group: bool,
) -> Result<Plan, ChannelError> {
    if in_group && visibility == Visibility::Private {
        return Ok(Plan {
            parts: vec![Part::Text(PRIVATE_IN_GROUP.to_owned())],
            keyboard: Vec::new(),
        });
    }
    let source = match &content.html {
        Some(html) => html.clone(),
        None => html_escape(content.plain.trim()),
    };
    let mut parts: Vec<Part> = split_messages(&html_to_telegram(&source))
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .map(Part::Text)
        .collect();
    for file in &content.attachments {
        if file.bytes.len() > MAX_UPLOAD_BYTES {
            parts.push(Part::Text(too_large_notice(&file.filename)));
        } else {
            parts.push(Part::File(file.clone()));
        }
    }
    if parts.is_empty() {
        return Err(ChannelError::Rejected("nothing to send".into()));
    }
    Ok(Plan {
        parts,
        keyboard: keyboard(content),
    })
}

fn too_large_notice(filename: &str) -> String {
    format!(
        "{} is larger than Telegram's {} MB upload limit and was not sent.",
        html_escape(filename),
        MAX_UPLOAD_BYTES / (1024 * 1024)
    )
}

/// T7: an over-long command is dropped, never truncated (a truncated command
/// could mean something else); a dropped button does not use up a slot.
fn keyboard(content: &Content) -> Vec<(String, String)> {
    content
        .actions
        .iter()
        .filter(|action| {
            let fits = !action.command.is_empty() && action.command.len() <= CALLBACK_DATA_MAX;
            if !fits {
                debug!(
                    bytes = action.command.len(),
                    "button command unusable; dropped"
                );
            }
            fits
        })
        .take(MAX_BUTTONS)
        .map(|action| {
            let label = action.label.trim();
            let label = if label.is_empty() {
                &action.command
            } else {
                label
            };
            (
                label.chars().take(LABEL_MAX).collect(),
                action.command.clone(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::message::{Action, Attachment, AttachmentKind, Content};

    fn file(kind: AttachmentKind, mime: &str, len: usize) -> Attachment {
        Attachment {
            kind,
            mime: mime.into(),
            filename: "f.bin".into(),
            bytes: vec![0; len],
            duration_ms: None,
            waveform: None,
        }
    }

    fn actions(n: usize) -> Vec<Action> {
        (0..n)
            .map(|i| Action {
                label: format!("Done #{i}"),
                command: format!("todo done {i}"),
            })
            .collect()
    }

    #[test]
    fn short_html_is_one_text_part() {
        let p = plan(
            &Content::html("<b>hi</b> &amp; bye"),
            Visibility::Normal,
            false,
        )
        .unwrap();
        assert_eq!(p.parts, vec![Part::Text("<b>hi</b> &amp; bye".into())]);
        assert!(p.keyboard.is_empty());
    }

    #[test]
    fn plain_text_is_escaped_not_interpreted() {
        let p = plan(&Content::text("a <b> & c"), Visibility::Normal, false).unwrap();
        assert_eq!(p.parts, vec![Part::Text("a &lt;b&gt; &amp; c".into())]);
    }

    #[test]
    fn long_text_is_split_under_the_limit() {
        let html = format!("<p>{}</p>", "x".repeat(9000));
        let p = plan(&Content::html(html), Visibility::Normal, false).unwrap();
        assert_eq!(p.parts.len(), 3);
    }

    #[test]
    fn private_reply_in_a_group_becomes_the_notice_only() {
        let mut c = Content::html("<b>secret</b>");
        c.actions = actions(2);
        c.attachments = vec![file(AttachmentKind::File, "text/plain", 3)];
        let p = plan(&c, Visibility::Private, true).unwrap();
        assert_eq!(p.parts, vec![Part::Text(PRIVATE_IN_GROUP.into())]);
        assert!(p.keyboard.is_empty());
    }

    #[test]
    fn private_reply_in_a_private_chat_is_full() {
        let p = plan(&Content::html("<b>secret</b>"), Visibility::Private, false).unwrap();
        assert_eq!(p.parts, vec![Part::Text("<b>secret</b>".into())]);
    }

    #[test]
    fn actions_become_a_capped_keyboard() {
        let mut c = Content::html("list");
        c.actions = actions(12);
        let p = plan(&c, Visibility::Normal, false).unwrap();
        assert_eq!(p.keyboard.len(), MAX_BUTTONS);
        assert_eq!(p.keyboard[0], ("Done #0".into(), "todo done 0".into()));
    }

    #[test]
    fn oversized_callback_data_drops_that_button_only() {
        let mut c = Content::html("list");
        c.actions = actions(2);
        c.actions[0].command = "x".repeat(CALLBACK_DATA_MAX + 1);
        c.actions[1].command = "é".repeat(CALLBACK_DATA_MAX / 2); // 64 bytes, 32 chars
        let p = plan(&c, Visibility::Normal, false).unwrap();
        assert_eq!(p.keyboard.len(), 1);
        assert_eq!(p.keyboard[0].1.len(), CALLBACK_DATA_MAX);
        // A dropped button does not use up one of the ten slots.
        let mut c = Content::html("list");
        c.actions = actions(11);
        c.actions[0].command = "x".repeat(65);
        assert_eq!(
            plan(&c, Visibility::Normal, false).unwrap().keyboard.len(),
            10
        );
    }

    #[test]
    fn long_labels_truncate_on_chars_and_blank_labels_fall_back() {
        let mut c = Content::html("list");
        c.actions = actions(2);
        c.actions[0].label = "é".repeat(100);
        c.actions[1].label = "  ".into();
        let p = plan(&c, Visibility::Normal, false).unwrap();
        assert_eq!(p.keyboard[0].0.chars().count(), 64);
        assert_eq!(p.keyboard[1].0, "todo done 1");
    }

    #[test]
    fn files_follow_the_caption_and_pick_the_send_method_by_kind() {
        let mut c = Content::html("caption");
        c.attachments = vec![file(AttachmentKind::Audio, "audio/ogg", 3)];
        let p = plan(&c, Visibility::Normal, false).unwrap();
        assert_eq!(p.parts.len(), 2);
        assert!(matches!(&p.parts[0], Part::Text(t) if t == "caption"));
        assert!(matches!(&p.parts[1], Part::File(a) if a.mime == "audio/ogg"));

        let m = |kind, mime| file_method(&file(kind, mime, 1));
        assert_eq!(m(AttachmentKind::Audio, "audio/ogg"), FileMethod::Voice);
        assert_eq!(
            m(AttachmentKind::Audio, "audio/ogg; codecs=opus"),
            FileMethod::Voice
        );
        assert_eq!(m(AttachmentKind::Audio, "audio/mpeg"), FileMethod::Audio);
        assert_eq!(m(AttachmentKind::Image, "image/png"), FileMethod::Photo);
        assert_eq!(
            m(AttachmentKind::File, "application/pdf"),
            FileMethod::Document
        );
    }

    #[test]
    fn a_file_over_the_cap_becomes_a_notice_not_an_error() {
        let mut c = Content::html("here");
        c.attachments = vec![
            file(AttachmentKind::File, "text/plain", MAX_UPLOAD_BYTES + 1),
            file(AttachmentKind::File, "text/plain", MAX_UPLOAD_BYTES),
        ];
        let p = plan(&c, Visibility::Normal, false).unwrap();
        assert_eq!(p.parts.len(), 3);
        assert!(matches!(&p.parts[1], Part::Text(t) if t.contains("f.bin") && t.contains("50 MB")));
        assert!(matches!(&p.parts[2], Part::File(_)));
    }

    #[test]
    fn nothing_to_send_is_rejected_not_an_empty_message() {
        let r = plan(&Content::text("  "), Visibility::Normal, false);
        assert!(matches!(r, Err(ChannelError::Rejected(_))));
    }
}
