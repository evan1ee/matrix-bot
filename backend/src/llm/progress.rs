//! What the agent tells a channel while a turn runs.
//!
//! The agent knows no channel: it reports stages, tool calls and streamed text
//! to a `ProgressSink`, and each adapter decides how to show them (Matrix edits
//! one message in place; a channel that cannot edit ignores most of it).

/// Receives a turn's progress. Every method is best-effort: a sink must never
/// fail the turn, so none returns an error.
#[async_trait::async_trait]
pub trait ProgressSink: Send + Sync {
    async fn report_stage(&self, text: &str);
    async fn report_tool_call(&self, tool_name: &str);
    /// Streaming text so far is appended; the sink throttles.
    async fn push_delta(&self, delta: &str);
    /// Marks the end of one streamed reply. A sink may send or discard what it
    /// buffered; either way the next stream starts from empty text.
    async fn flush(&self);
    /// Adapter message id of the progress message, if any (Matrix: the stage event id).
    fn message_ref(&self) -> Option<String> {
        None
    }
}

/// A turn nobody watches.
pub struct NoProgress;

#[async_trait::async_trait]
impl ProgressSink for NoProgress {
    async fn report_stage(&self, _text: &str) {}
    async fn report_tool_call(&self, _tool_name: &str) {}
    async fn push_delta(&self, _delta: &str) {}
    async fn flush(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn no_progress_accepts_everything_silently() {
        let p = NoProgress;
        p.report_stage("x").await;
        p.report_tool_call("todo_add").await;
        p.push_delta("a").await;
        p.flush().await;
        assert_eq!(p.message_ref(), None);
    }
}
