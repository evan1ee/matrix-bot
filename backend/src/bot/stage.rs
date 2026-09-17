use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use matrix_sdk::ruma::events::room::message::{ReplacementMetadata, RoomMessageEventContent};
use matrix_sdk::ruma::OwnedEventId;
use matrix_sdk::Room;
use tokio::task::JoinHandle;
use tracing::{debug, error, warn};

/// A background task that keeps the Matrix room typing notification alive
/// by continuously pulsing `room.typing_notice(true)` until stopped.
pub struct TypingGuard {
    active: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    room: Room,
}

impl TypingGuard {
    /// Spawns a background heartbeat pulsing `typing_notice(true)` every 3 seconds.
    pub fn start(room: Room) -> Self {
        let active = Arc::new(AtomicBool::new(true));
        let active_clone = active.clone();
        let room_clone = room.clone();

        let handle = tokio::spawn(async move {
            let _ = room_clone.typing_notice(true).await;

            while active_clone.load(Ordering::Relaxed) {
                tokio::time::sleep(Duration::from_secs(3)).await;
                if !active_clone.load(Ordering::Relaxed) {
                    break;
                }
                let _ = room_clone.typing_notice(true).await;
            }
        });

        Self {
            active,
            handle: Some(handle),
            room,
        }
    }

    /// Stops the typing heartbeat and clears the typing notice in the room.
    pub async fn stop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        let _ = self.room.typing_notice(false).await;
    }
}

impl Drop for TypingGuard {
    fn drop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        let room = self.room.clone();
        tokio::spawn(async move {
            let _ = room.typing_notice(false).await;
        });
    }
}

/// Lightweight handle that allows worker subsystems (like AI tool execution)
/// to publish interim stage progress into the in-place status message.
#[derive(Clone)]
pub struct StageReporter {
    room: Option<Room>,
    event_id: Option<OwnedEventId>,
}

impl std::fmt::Debug for StageReporter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let room_id = self
            .room
            .as_ref()
            .map(|r| r.room_id().as_str())
            .unwrap_or("none");
        f.debug_struct("StageReporter")
            .field("room_id", &room_id)
            .field("event_id", &self.event_id)
            .finish()
    }
}

impl StageReporter {
    pub fn noop() -> Self {
        Self {
            room: None,
            event_id: None,
        }
    }

    pub async fn report(&self, plain_text: &str, html_text: &str) {
        if let (Some(room), Some(target_id)) = (&self.room, &self.event_id) {
            let metadata = ReplacementMetadata::new(target_id.clone(), None);
            let content = RoomMessageEventContent::text_html(plain_text, html_text)
                .make_replacement(metadata, None);
            if let Err(err) = room.send(content).await {
                debug!(
                    "Failed to update stage indicator in room {}: {:?}",
                    room.room_id(),
                    err
                );
            }
        }
    }

    pub async fn report_stage(&self, text: &str) {
        let html = format!("⏳ <i>{}</i>", crate::utils::html::html_escape(text));
        self.report(text, &html).await;
    }

    pub async fn report_tool_call(&self, tool_name: &str) {
        let label = tool_to_stage_label(tool_name);
        self.report_stage(label).await;
    }

    /// Returns the target Matrix event ID if available.
    pub fn event_id(&self) -> Option<&OwnedEventId> {
        self.event_id.as_ref()
    }

    /// Creates a throttled streaming buffer for in-place token replacement.
    pub fn throttler(&self) -> StreamThrottler {
        StreamThrottler::new(self.clone())
    }
}

/// Buffers and throttles streaming token edits (`m.replace`) to a Matrix room.
///
/// Enforces a minimum interval (default 800ms) and character threshold between
/// message edits, protecting against homeserver (Synapse) HTTP 429 rate limits
/// while delivering a smooth typewriter animation in Element X.
pub struct StreamThrottler {
    reporter: StageReporter,
    accumulated: String,
    last_sent_at: std::time::Instant,
    last_sent_len: usize,
    min_interval: Duration,
    min_chars: usize,
}

impl StreamThrottler {
    pub fn new(reporter: StageReporter) -> Self {
        Self {
            reporter,
            accumulated: String::new(),
            last_sent_at: std::time::Instant::now(),
            last_sent_len: 0,
            min_interval: Duration::from_millis(800),
            min_chars: 15,
        }
    }

    pub fn with_interval(mut self, interval: Duration) -> Self {
        self.min_interval = interval;
        self
    }

    pub async fn push_delta(&mut self, delta: &str) {
        self.accumulated.push_str(delta);
        let new_chars = self.accumulated.len().saturating_sub(self.last_sent_len);

        if self.last_sent_at.elapsed() >= self.min_interval && new_chars >= self.min_chars {
            self.flush().await;
        }
    }

    pub async fn flush(&mut self) {
        if self.accumulated.is_empty() {
            return;
        }
        self.last_sent_at = std::time::Instant::now();
        self.last_sent_len = self.accumulated.len();

        let plain = format!("{} ...", self.accumulated);
        let html = format!("{} ▍", crate::utils::html::html_escape(&self.accumulated));
        self.reporter.report(&plain, &html).await;
    }

    pub fn accumulated(&self) -> &str {
        &self.accumulated
    }
}

/// Manages an in-place updating status message in a Matrix room.
/// Starts with an initial stage placeholder, edits it as preparation advances,
/// and replaces it in-place with the final rich reply via `m.replace`.
pub struct StageIndicator {
    event_id: Option<OwnedEventId>,
    room: Room,
}

impl StageIndicator {
    /// Sends the initial placeholder message and records its `EventId`.
    pub async fn start(room: &Room, plain_text: &str, html_text: &str) -> Self {
        let content = RoomMessageEventContent::text_html(plain_text, html_text);
        let event_id = match room.send(content).await {
            Ok(resp) => Some(resp.event_id),
            Err(err) => {
                warn!(
                    "Failed to post initial stage indicator in room {}: {:?}",
                    room.room_id(),
                    err
                );
                None
            }
        };

        Self {
            event_id,
            room: room.clone(),
        }
    }

    /// Returns the target Matrix event ID if available.
    pub fn event_id(&self) -> Option<&OwnedEventId> {
        self.event_id.as_ref()
    }

    /// Produces a cloneable reporter handle for subroutines.
    pub fn reporter(&self) -> StageReporter {
        StageReporter {
            room: Some(self.room.clone()),
            event_id: self.event_id.clone(),
        }
    }

    /// Updates the placeholder message in-place to reflect a new progress stage.
    pub async fn update_stage(&self, plain_text: &str, html_text: &str) {
        if let Some(ref target_id) = self.event_id {
            let metadata = ReplacementMetadata::new(target_id.clone(), None);
            let content = RoomMessageEventContent::text_html(plain_text, html_text)
                .make_replacement(metadata, None);
            if let Err(err) = self.room.send(content).await {
                debug!(
                    "Failed to update stage indicator in room {}: {:?}",
                    self.room.room_id(),
                    err
                );
            }
        }
    }

    /// Replaces the placeholder message in-place with the final rich reply.
    pub async fn finish_with_reply(&self, plain_text: &str, html_reply: &str) -> Option<OwnedEventId> {
        if let Some(ref target_id) = self.event_id {
            let metadata = ReplacementMetadata::new(target_id.clone(), None);
            let content = RoomMessageEventContent::text_html(plain_text, html_reply)
                .make_replacement(metadata, None);
            match self.room.send(content).await {
                Ok(resp) => Some(resp.event_id),
                Err(err) => {
                    error!(
                        "Failed to replace stage indicator with final reply in room {}: {:?}",
                        self.room.room_id(),
                        err
                    );
                    None
                }
            }
        } else {
            let content = RoomMessageEventContent::text_html(plain_text, html_reply);
            match self.room.send(content).await {
                Ok(resp) => Some(resp.event_id),
                Err(err) => {
                    error!(
                        "Failed to send final reply to room {}: {:?}",
                        self.room.room_id(),
                        err
                    );
                    None
                }
            }
        }
    }

    /// Replaces the placeholder message in-place with an error message.
    pub async fn cancel_with_error(&self, plain_error: &str, html_error: &str) {
        if let Some(ref target_id) = self.event_id {
            let metadata = ReplacementMetadata::new(target_id.clone(), None);
            let content = RoomMessageEventContent::text_html(plain_error, html_error)
                .make_replacement(metadata, None);
            let _ = self.room.send(content).await;
        } else {
            let content = RoomMessageEventContent::text_html(plain_error, html_error);
            let _ = self.room.send(content).await;
        }
    }
}

/// Maps function call tool names to friendly localized stage labels.
pub fn tool_to_stage_label(tool_name: &str) -> &'static str {
    match tool_name {
        "record_finance_entry" | "create_recurring_bill" => "⚡ 正在记录支出...",
        "set_reminder" | "create_reminder" => "⚡ 正在设定提醒...",
        "add_todo" | "complete_todo" => "⚡ 正在更新待办事项...",
        "search_web" => "🌐 正在联网搜索验证...",
        "log_habit" => "🔥 正在打卡记录习惯...",
        "log_footprint" => "📍 正在记录足迹...",
        "create_bookmark" => "🔖 正在保存书签...",
        _ => "⚡ 正在执行操作...",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use matrix_sdk::ruma::EventId;

    #[test]
    fn test_tool_to_stage_label_mappings() {
        assert_eq!(tool_to_stage_label("record_finance_entry"), "⚡ 正在记录支出...");
        assert_eq!(tool_to_stage_label("set_reminder"), "⚡ 正在设定提醒...");
        assert_eq!(tool_to_stage_label("search_web"), "🌐 正在联网搜索验证...");
        assert_eq!(tool_to_stage_label("add_todo"), "⚡ 正在更新待办事项...");
        assert_eq!(tool_to_stage_label("unknown_tool"), "⚡ 正在执行操作...");
    }

    #[test]
    fn test_stage_replacement_content_construction() {
        let target_id = EventId::parse("$stage_event:example.org").unwrap();
        let metadata = ReplacementMetadata::new(target_id.clone(), None);
        let content = RoomMessageEventContent::text_html("Listening...", "<i>Listening...</i>")
            .make_replacement(metadata, None);

        assert!(content.relates_to.is_some());
    }

    #[tokio::test]
    async fn test_stream_throttler_accumulation() {
        let reporter = StageReporter::noop();
        let mut throttler = reporter.throttler().with_interval(Duration::from_millis(50));
        assert_eq!(throttler.accumulated(), "");

        throttler.push_delta("Hello").await;
        throttler.push_delta(" world! This is a longer stream test.").await;
        assert!(throttler.accumulated().contains("Hello world!"));

        throttler.flush().await;
        assert_eq!(throttler.accumulated(), "Hello world! This is a longer stream test.");
    }
}
