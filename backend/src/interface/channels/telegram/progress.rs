//! What the agent's progress looks like on Telegram (T15): the typing action,
//! and one status message that is edited in place and deleted at the end.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use teloxide_core::prelude::*;
use teloxide_core::types::{ChatAction, ChatId, MessageId};
use tracing::debug;

use super::outbound::bounded;
use crate::interface::channels::discord::progress::Throttle;
use crate::llm::progress::ProgressSink;

/// Telegram shows "typing" for about five seconds.
const TYPING_GAP: Duration = Duration::from_secs(4);
/// Edits of one message are rate limited; stay well inside.
const EDIT_GAP: Duration = Duration::from_millis(1500);
/// A message holds 4096 characters; leave room.
const STATUS_MAX: usize = 4000;

#[derive(Debug, PartialEq, Eq)]
enum StatusStep {
    Send(String),
    Edit(MessageId, String),
}

#[derive(Debug, PartialEq, Eq)]
struct Step {
    typing: bool,
    status: Option<StatusStep>,
}

enum Status {
    None,
    /// The first status message is on its way; nothing to edit yet.
    Sending,
    Posted(MessageId),
}

struct State {
    typing: Throttle,
    edits: Throttle,
    status: Status,
    streamed: String,
}

impl Default for State {
    fn default() -> Self {
        Self {
            typing: Throttle::new(TYPING_GAP),
            edits: Throttle::new(EDIT_GAP),
            status: Status::None,
            streamed: String::new(),
        }
    }
}

impl State {
    /// What one report calls for. `text` is the status to show, if any (a bare
    /// pulse passes `None`). Pure: the clock is a parameter.
    fn next(&mut self, now: Instant, text: Option<String>) -> Step {
        let typing = self.typing.ready(now);
        let status = match (text, &self.status) {
            (Some(text), Status::None) => {
                self.status = Status::Sending;
                self.edits.ready(now);
                Some(StatusStep::Send(text))
            }
            (Some(text), Status::Posted(id)) if self.edits.ready(now) => {
                Some(StatusStep::Edit(*id, text))
            }
            _ => None,
        };
        Step { typing, status }
    }

    fn sent(&mut self, id: MessageId) {
        self.status = Status::Posted(id);
    }

    fn send_failed(&mut self) {
        self.status = Status::None;
    }

    fn take_status(&mut self) -> Option<MessageId> {
        match std::mem::replace(&mut self.status, Status::None) {
            Status::Posted(id) => Some(id),
            other => {
                self.status = other;
                None
            }
        }
    }
}

/// The last `max` characters of `text`.
fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(max)).collect()
}

/// Shows a turn's progress in one chat. Failures are logged at debug and
/// ignored: a sink never fails the turn.
pub struct TelegramProgress {
    bot: Bot,
    chat: ChatId,
    state: Mutex<State>,
}

impl TelegramProgress {
    pub fn new(bot: Bot, chat: ChatId) -> Self {
        Self {
            bot,
            chat,
            state: Mutex::new(State::default()),
        }
    }

    async fn report(&self, text: Option<String>) {
        let step = match self.state.lock() {
            Ok(mut state) => state.next(Instant::now(), text),
            Err(_) => return,
        };
        if step.typing {
            let action = self.bot.send_chat_action(self.chat, ChatAction::Typing);
            if let Err(err) = bounded(action.send()).await {
                debug!(error = %err, "Telegram typing action failed");
            }
        }
        match step.status {
            Some(StatusStep::Send(text)) => {
                match bounded(self.bot.send_message(self.chat, text).send()).await {
                    Ok(message) => self.update(|s| s.sent(message.id)),
                    Err(err) => {
                        debug!(error = %err, "Telegram status message failed");
                        self.update(State::send_failed);
                    }
                }
            }
            Some(StatusStep::Edit(id, text)) => {
                let edit = self.bot.edit_message_text(self.chat, id, text);
                if let Err(err) = bounded(edit.send()).await {
                    debug!(error = %err, "Telegram status edit failed");
                }
            }
            None => {}
        }
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        if let Ok(mut state) = self.state.lock() {
            change(&mut state);
        }
    }

    /// Removes the status message once the reply is delivered; when it cannot
    /// be deleted it is edited to a check mark instead.
    pub async fn finish(&self) {
        let Some(id) = self.state.lock().ok().and_then(|mut s| s.take_status()) else {
            return;
        };
        if let Err(err) = bounded(self.bot.delete_message(self.chat, id).send()).await {
            debug!(error = %err, "Telegram status delete failed");
            let edit = self.bot.edit_message_text(self.chat, id, "✓");
            let _ = bounded(edit.send()).await;
        }
    }
}

#[async_trait::async_trait]
impl ProgressSink for TelegramProgress {
    async fn report_stage(&self, text: &str) {
        self.report(Some(format!("⏳ {text}"))).await;
    }

    async fn report_tool_call(&self, tool_name: &str) {
        self.report(Some(format!("⏳ {tool_name}"))).await;
    }

    async fn push_delta(&self, delta: &str) {
        let shown = match self.state.lock() {
            Ok(mut state) => {
                state.streamed.push_str(delta);
                tail(&state.streamed, STATUS_MAX)
            }
            Err(_) => return,
        };
        self.report(Some(shown)).await;
    }

    async fn flush(&self) {
        self.update(|s| s.streamed.clear());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(ms: u64, start: Instant) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn the_first_report_sends_a_status_message_and_a_typing_action() {
        let t0 = Instant::now();
        let mut s = State::default();
        let step = s.next(at(0, t0), Some("⏳ thinking".into()));
        assert!(step.typing);
        assert_eq!(step.status, Some(StatusStep::Send("⏳ thinking".into())));
    }

    #[test]
    fn typing_is_at_most_every_four_seconds() {
        let t0 = Instant::now();
        let mut s = State::default();
        assert!(s.next(at(0, t0), None).typing);
        assert!(!s.next(at(3_900, t0), None).typing);
        assert!(s.next(at(4_000, t0), None).typing);
    }

    #[test]
    fn edits_wait_for_the_status_id_and_are_throttled() {
        let t0 = Instant::now();
        let mut s = State::default();
        s.next(at(0, t0), Some("a".into()));
        // The send is still in flight: nothing to edit yet.
        assert_eq!(s.next(at(2_000, t0), Some("b".into())).status, None);
        s.sent(MessageId(7));
        let step = s.next(at(2_100, t0), Some("c".into()));
        assert_eq!(
            step.status,
            Some(StatusStep::Edit(MessageId(7), "c".into()))
        );
        assert_eq!(s.next(at(3_000, t0), Some("d".into())).status, None);
        assert!(matches!(
            s.next(at(3_700, t0), Some("e".into())).status,
            Some(StatusStep::Edit(MessageId(7), _))
        ));
    }

    #[test]
    fn a_failed_send_allows_a_new_status_message() {
        let t0 = Instant::now();
        let mut s = State::default();
        s.next(at(0, t0), Some("a".into()));
        s.send_failed();
        assert!(matches!(
            s.next(at(2_000, t0), Some("b".into())).status,
            Some(StatusStep::Send(_))
        ));
    }

    #[test]
    fn finishing_hands_back_the_status_message_once() {
        let mut s = State::default();
        assert_eq!(s.take_status(), None);
        s.sent(MessageId(9));
        assert_eq!(s.take_status(), Some(MessageId(9)));
        assert_eq!(s.take_status(), None);
    }

    #[test]
    fn streamed_text_is_tail_truncated() {
        assert_eq!(tail("abcdef", 3), "def");
        assert_eq!(tail("ab", 3), "ab");
        assert_eq!(tail("字字字", 2), "字字");
    }
}
