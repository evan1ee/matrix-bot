//! What the agent's progress looks like on Discord: edits of the deferred
//! interaction response, or the typing indicator in a DM.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serenity::all::{ChannelId, CommandInteraction, Http};
use tracing::debug;

use super::outbound::{bounded, edit_response};
use super::render::{Post, EMBED_DESCRIPTION_MAX};
use crate::interface::channels::render::markdown::escape_markdown;
use crate::interface::channels::throttle::Throttle;
use crate::llm::progress::ProgressSink;

/// Discord allows about five interaction edits per five seconds.
pub const EDIT_GAP: Duration = Duration::from_millis(1500);
/// The typing indicator lasts ten seconds; pulse a little sooner.
pub const TYPING_GAP: Duration = Duration::from_secs(8);

struct Edits {
    throttle: Throttle,
    streamed: String,
}

impl Edits {
    /// Appends a streamed delta and returns the text to show.
    fn stream(&mut self, delta: &str) -> String {
        self.streamed.push_str(delta);
        tail(&self.streamed, EMBED_DESCRIPTION_MAX)
    }

    /// Ends the current segment, sent or not: the next delta starts from empty text.
    fn end_segment(&mut self) {
        self.streamed.clear();
    }
}

/// Edits the deferred response while a turn runs; the final reply replaces it.
/// Failures are logged at debug and ignored: a sink never fails the turn.
pub struct InteractionProgress {
    http: Arc<Http>,
    interaction: Arc<CommandInteraction>,
    state: Mutex<Edits>,
}

impl InteractionProgress {
    pub fn new(http: Arc<Http>, interaction: Arc<CommandInteraction>) -> Self {
        Self {
            http,
            interaction,
            state: Mutex::new(Edits {
                throttle: Throttle::new(EDIT_GAP),
                streamed: String::new(),
            }),
        }
    }

    /// `next` decides, under the lock, whether to edit and with what text.
    async fn edit_if_due(&self, next: impl FnOnce(&mut Edits) -> Option<String>) {
        let text = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            let due = state.throttle.ready(Instant::now());
            let text = next(&mut state);
            if !due {
                return;
            }
            text
        };
        let Some(text) = text else {
            return;
        };
        let edit = self
            .interaction
            .edit_response(&self.http, edit_response(Post::text(text)));
        if let Err(err) = bounded(edit).await {
            debug!(error = %err, "Discord progress edit failed");
        }
    }
}

/// The last `max` characters of `text`.
fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max)).collect()
}

#[async_trait::async_trait]
impl ProgressSink for InteractionProgress {
    async fn report_stage(&self, text: &str) {
        let line = format!("⏳ {}", escape_markdown(text));
        self.edit_if_due(|_| Some(line)).await;
    }

    async fn report_tool_call(&self, tool_name: &str) {
        let line = format!("⏳ {}", escape_markdown(tool_name));
        self.edit_if_due(|_| Some(line)).await;
    }

    async fn push_delta(&self, delta: &str) {
        self.edit_if_due(|state| Some(state.stream(delta))).await;
    }

    async fn flush(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.end_segment();
        }
    }

    /// Drops the segment without an edit of its own; the fallback's text
    /// replaces it on the next due edit.
    async fn retract_segment(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.end_segment();
        }
    }
}

/// A DM shows only the typing indicator while the agent works.
pub struct TypingProgress {
    http: Arc<Http>,
    channel: ChannelId,
    throttle: Mutex<Throttle>,
}

impl TypingProgress {
    pub fn new(http: Arc<Http>, channel: ChannelId) -> Self {
        Self {
            http,
            channel,
            throttle: Mutex::new(Throttle::new(TYPING_GAP)),
        }
    }

    async fn pulse(&self) {
        let due = self
            .throttle
            .lock()
            .map(|mut t| t.ready(Instant::now()))
            .unwrap_or(false);
        if due {
            if let Err(err) = bounded(self.channel.broadcast_typing(&self.http)).await {
                debug!(error = %err, "Discord typing indicator failed");
            }
        }
    }
}

#[async_trait::async_trait]
impl ProgressSink for TypingProgress {
    async fn report_stage(&self, _text: &str) {
        self.pulse().await;
    }
    async fn report_tool_call(&self, _tool_name: &str) {
        self.pulse().await;
    }
    async fn push_delta(&self, _delta: &str) {
        self.pulse().await;
    }
    async fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_retracted_segment_is_not_carried_into_the_fallback_text() {
        let mut edits = Edits {
            throttle: Throttle::new(EDIT_GAP),
            streamed: String::new(),
        };
        edits.stream("partial");
        edits.end_segment();
        assert_eq!(edits.stream("Full answer"), "Full answer");
    }

    #[test]
    fn tail_keeps_the_newest_characters() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
        assert_eq!(tail("字字字", 2), "字字");
    }
}
