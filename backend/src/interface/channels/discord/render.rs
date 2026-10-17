//! Neutral content → Discord posts, and where an interaction reply goes. Pure.

use tracing::debug;

use super::commands::DeferMode;
use crate::interface::channels::adapter::ChannelError;
use crate::interface::channels::message::{Attachment, Content, Visibility};
use crate::interface::channels::render::markdown::{escape_markdown, html_to_markdown};

/// Discord's embed description limit, in characters.
pub const EMBED_DESCRIPTION_MAX: usize = 4096;
/// Discord's default upload limit (B14).
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
/// What a public guild response says when its content had to go privately (B2).
pub const PRIVATE_NOTICE: &str = "Sent privately.";
/// What a deferred interaction says when nothing answered it (B11).
pub const NO_REPLY: &str = "Nothing to answer for that.";

const FENCE: &str = "```";

/// One Discord message: one embed description and, on the last post only, the files and buttons.
#[derive(Debug, PartialEq)]
pub struct Post {
    pub description: Option<String>,
    pub files: Vec<Attachment>,
    pub buttons: Vec<(String, String)>,
}

impl Post {
    pub fn text(description: impl Into<String>) -> Self {
        Self {
            description: Some(description.into()),
            files: Vec::new(),
            buttons: Vec::new(),
        }
    }
}

/// `Content` → posts: Markdown from `html` (else the escaped `plain`), split at
/// line boundaries into embed-sized chunks, files and buttons on the last post.
/// Empty content without files or buttons becomes [`NO_REPLY`].
pub fn posts(content: &Content) -> Result<Vec<Post>, ChannelError> {
    if let Some(big) = content
        .attachments
        .iter()
        .find(|a| a.bytes.len() > MAX_UPLOAD_BYTES)
    {
        return Err(ChannelError::Rejected(format!(
            "{} is larger than Discord's {} MiB upload limit",
            big.filename,
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )));
    }
    let buttons: Vec<(String, String)> = content
        .actions
        .iter()
        .filter_map(|action| {
            if action.command.is_empty() {
                return None;
            }
            if action.command.len() > 100 {
                debug!(
                    command = action.command,
                    "Discord button custom_id exceeds 100 bytes; dropped"
                );
                return None;
            }
            let label: String = action.label.trim().chars().take(80).collect();
            if label.is_empty() {
                return None;
            }
            Some((label, action.command.clone()))
        })
        .take(25)
        .collect();

    let markdown = match &content.html {
        Some(html) => html_to_markdown(html),
        None => escape_markdown(content.plain.trim()),
    };
    let mut chunks: Vec<Option<String>> = split(&markdown, EMBED_DESCRIPTION_MAX)
        .into_iter()
        .map(Some)
        .collect();
    if chunks.is_empty() {
        chunks.push(
            (content.attachments.is_empty() && buttons.is_empty())
                .then(|| NO_REPLY.to_owned()),
        );
    }
    let last = chunks.len() - 1;
    Ok(chunks
        .into_iter()
        .enumerate()
        .map(|(i, description)| Post {
            description,
            files: if i == last {
                content.attachments.clone()
            } else {
                Vec::new()
            },
            buttons: if i == last {
                buttons.clone()
            } else {
                Vec::new()
            },
        })
        .collect())
}

#[derive(Debug, PartialEq, Eq)]
pub enum Route {
    /// Edit the deferred response with the first post; the rest are follow-ups
    /// with the same ephemerality as the defer.
    EditOriginal,
    /// Edit the public response to [`PRIVATE_NOTICE`]; every post goes out as an
    /// ephemeral follow-up.
    PrivateFollowUp,
}

/// Spec invariant 10: a private reply never lands on a public guild response.
pub fn route_reply(visibility: Visibility, in_guild: bool, deferred: DeferMode) -> Route {
    match (visibility, in_guild, deferred) {
        (Visibility::Private, true, DeferMode::Public) => Route::PrivateFollowUp,
        _ => Route::EditOriginal,
    }
}

/// Splits `text` into chunks of at most `max` characters, at line boundaries
/// when it can; a longer line is cut on a char boundary. A code fence open at a
/// cut is closed there and reopened in the next chunk.
fn split(text: &str, max: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return Vec::new();
    }
    // Room for a closing fence (newline + ```) and a reopening one on the next chunk.
    let budget = max.saturating_sub(FENCE.len() * 2 + 2).max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    let mut in_fence = false;

    let flush = |chunks: &mut Vec<String>, current: &mut String, in_fence: bool| {
        let mut done = std::mem::take(current);
        if in_fence {
            done.push('\n');
            done.push_str(FENCE);
        }
        chunks.push(done);
        if in_fence {
            current.push_str(FENCE);
            current.push('\n');
        }
    };

    for line in text.split('\n') {
        let mut line_chars: Vec<char> = line.chars().collect();
        loop {
            let separator = usize::from(current_len > 0);
            let room = budget.saturating_sub(current_len + separator);
            if line_chars.len() <= room {
                if separator == 1 {
                    current.push('\n');
                }
                current.extend(line_chars.iter());
                current_len += separator + line_chars.len();
                break;
            }
            if current_len > 0 && line_chars.len() <= budget {
                // The whole line fits a fresh chunk: cut before it.
                flush(&mut chunks, &mut current, in_fence);
                current_len = current.chars().count();
                continue;
            }
            // A line longer than a chunk: fill this one and carry the rest.
            let take = room.max(1).min(line_chars.len());
            if separator == 1 && room > 0 {
                current.push('\n');
            }
            current.extend(line_chars.drain(..take));
            flush(&mut chunks, &mut current, in_fence);
            current_len = current.chars().count();
        }
        if line.trim_start().starts_with(FENCE) {
            in_fence = !in_fence;
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::message::{Action, AttachmentKind};

    fn audio(len: usize) -> Attachment {
        Attachment {
            kind: AttachmentKind::Audio,
            mime: "audio/ogg".into(),
            filename: "voice.ogg".into(),
            bytes: vec![0; len],
            duration_ms: None,
            waveform: None,
        }
    }

    #[test]
    fn short_html_is_one_markdown_post() {
        let posts = posts(&Content::html("<b>Done</b>")).unwrap();
        assert_eq!(posts, vec![Post::text("**Done**")]);
    }

    #[test]
    fn plain_content_is_escaped() {
        let posts = posts(&Content::text("a*b")).unwrap();
        assert_eq!(posts, vec![Post::text("a\\*b")]);
    }

    #[test]
    fn long_text_splits_at_lines_within_the_embed_limit() {
        let line = "y".repeat(89);
        let text = vec![line.as_str(); 100].join("\n"); // 9 000 characters
        let posts = posts(&Content::text(text.clone())).unwrap();
        assert_eq!(posts.len(), 3);
        for post in &posts {
            let d = post.description.as_ref().unwrap();
            assert!(d.chars().count() <= EMBED_DESCRIPTION_MAX);
            assert!(d.split('\n').all(|l| l == line), "a line was cut");
        }
        let joined: Vec<String> = posts.into_iter().filter_map(|p| p.description).collect();
        assert_eq!(joined.join("\n"), text);
    }

    #[test]
    fn a_single_long_line_is_cut_on_a_char_boundary() {
        let text = "字".repeat(5000);
        let posts = posts(&Content::text(text.clone())).unwrap();
        assert_eq!(posts.len(), 2);
        let joined: String = posts.into_iter().filter_map(|p| p.description).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn a_fence_crossing_a_cut_stays_fenced_on_both_sides() {
        let body = vec!["code line"; 600].join("\n");
        let posts = posts(&Content::html(format!("<pre>{body}</pre>"))).unwrap();
        assert!(posts.len() >= 2);
        for post in &posts {
            let d = post.description.as_ref().unwrap();
            assert!(d.chars().count() <= EMBED_DESCRIPTION_MAX);
            assert!(d.starts_with(FENCE), "{}", &d[..20]);
            assert!(d.ends_with(FENCE));
        }
    }

    #[test]
    fn files_ride_on_the_last_post_and_oversized_ones_are_rejected() {
        let mut content = Content::text("x\n".repeat(3000));
        content.attachments.push(audio(10));
        let posts = posts(&content).unwrap();
        assert!(posts[..posts.len() - 1].iter().all(|p| p.files.is_empty()));
        assert_eq!(posts.last().unwrap().files.len(), 1);

        let mut big = Content::text("x");
        big.attachments.push(audio(MAX_UPLOAD_BYTES + 1));
        assert!(matches!(super::posts(&big), Err(ChannelError::Rejected(_))));
    }

    #[test]
    fn an_attachment_alone_has_no_text_and_empty_content_says_so() {
        let mut only_file = Content::default();
        only_file.attachments.push(audio(1));
        let posts = posts(&only_file).unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].description, None);
        assert_eq!(
            super::posts(&Content::default()).unwrap(),
            vec![Post::text(NO_REPLY)]
        );
    }

    #[test]
    fn posts_render_actions_as_buttons() {
        let mut content = Content::text("Your tasks");
        content.actions = vec![Action {
            label: "Done #1".into(),
            command: "todo done 1".into(),
        }];
        let rendered = posts(&content).unwrap();
        assert_eq!(rendered[0].buttons, vec![("Done #1".into(), "todo done 1".into())]);
    }

    #[test]
    fn button_rendering_applies_limits_and_filtering() {
        let mut content = Content::text("Tasks");
        // 0. whitespace-only label -> dropped
        content.actions.push(Action {
            label: "   \t\n".into(),
            command: "todo done 99".into(),
        });
        // 1. empty command -> dropped
        content.actions.push(Action {
            label: "Empty".into(),
            command: "".into(),
        });
        // 2. command > 100 bytes -> dropped
        content.actions.push(Action {
            label: "Overlong".into(),
            command: "a".repeat(101),
        });
        // 3. label > 80 chars -> truncated
        content.actions.push(Action {
            label: "l".repeat(100),
            command: "todo done 1".into(),
        });
        // 4. exactly 100 bytes command -> kept
        content.actions.push(Action {
            label: "Exact 100".into(),
            command: "b".repeat(100),
        });
        // 5. add 30 more valid actions to test 25 limit
        for i in 0..30 {
            content.actions.push(Action {
                label: format!("Task {i}"),
                command: format!("todo done {i}"),
            });
        }
        let rendered = posts(&content).unwrap();
        assert_eq!(rendered[0].buttons.len(), 25);
        assert_eq!(rendered[0].buttons[0].0, "l".repeat(80));
        assert_eq!(rendered[0].buttons[0].1, "todo done 1");
        assert_eq!(rendered[0].buttons[1].0, "Exact 100");
        assert_eq!(rendered[0].buttons[1].1, "b".repeat(100));
        assert_eq!(rendered[0].buttons[2].0, "Task 0");
    }

    #[test]
    fn buttons_ride_on_the_last_post_only() {
        let mut content = Content::text("x\n".repeat(3000));
        content.actions.push(Action {
            label: "Click".into(),
            command: "help".into(),
        });
        let rendered = posts(&content).unwrap();
        assert!(rendered.len() > 1);
        assert!(rendered[..rendered.len() - 1]
            .iter()
            .all(|p| p.buttons.is_empty()));
        assert_eq!(
            rendered.last().unwrap().buttons,
            vec![("Click".into(), "help".into())]
        );
    }

    #[test]
    fn buttons_alone_have_no_text_and_no_reply_notice() {
        let mut content = Content::default();
        content.actions.push(Action {
            label: "Click".into(),
            command: "help".into(),
        });
        let rendered = posts(&content).unwrap();
        assert_eq!(rendered.len(), 1);
        assert_eq!(rendered[0].description, None);
        assert_eq!(rendered[0].buttons, vec![("Click".into(), "help".into())]);
    }

    #[test]
    fn only_a_private_reply_on_a_public_guild_response_goes_privately() {
        for visibility in [Visibility::Normal, Visibility::Private] {
            for in_guild in [true, false] {
                for mode in [DeferMode::Public, DeferMode::Ephemeral] {
                    let expected = if visibility == Visibility::Private
                        && in_guild
                        && mode == DeferMode::Public
                    {
                        Route::PrivateFollowUp
                    } else {
                        Route::EditOriginal
                    };
                    assert_eq!(
                        route_reply(visibility, in_guild, mode),
                        expected,
                        "{visibility:?} guild={in_guild} {mode:?}"
                    );
                }
            }
        }
    }
}
