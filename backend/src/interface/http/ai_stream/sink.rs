//! Turns a turn's progress into stream events on a bounded channel.

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::events::AiStreamEvent;
use crate::domain::llm::ToolStepOutcome;
use crate::llm::progress::ProgressSink;

/// Sends every event with `send(..).await`, so a full channel slows the turn
/// (backpressure) and no event is ever dropped. When the receiver is gone the
/// client has left, so the sink cancels the turn's token.
pub struct WebStreamSink {
    tx: mpsc::Sender<AiStreamEvent>,
    cancel: CancellationToken,
}

impl WebStreamSink {
    pub fn new(tx: mpsc::Sender<AiStreamEvent>, cancel: CancellationToken) -> Self {
        Self { tx, cancel }
    }

    async fn emit(&self, event: AiStreamEvent) {
        if self.tx.send(event).await.is_err() {
            self.cancel.cancel();
        }
    }
}

#[async_trait::async_trait]
impl ProgressSink for WebStreamSink {
    // The web UI draws its own "Thinking…" state from the event sequence.
    async fn report_stage(&self, _text: &str) {}
    // Superseded by `report_tool_started` / `report_tool_finished`.
    async fn report_tool_call(&self, _tool_name: &str) {}

    async fn push_delta(&self, delta: &str) {
        self.emit(AiStreamEvent::Delta {
            text: delta.to_string(),
        })
        .await;
    }

    async fn flush(&self) {
        self.emit(AiStreamEvent::SegmentEnd).await;
    }

    async fn report_conversation(&self, conversation_id: &str) {
        self.emit(AiStreamEvent::Started {
            conversation_id: conversation_id.to_string(),
        })
        .await;
    }

    async fn report_tool_started(&self, call_id: &str, name: &str) {
        self.emit(AiStreamEvent::ToolStarted {
            call_id: call_id.to_string(),
            name: name.to_string(),
        })
        .await;
    }

    async fn report_tool_finished(&self, call_id: &str, name: &str, outcome: ToolStepOutcome) {
        self.emit(AiStreamEvent::ToolFinished {
            call_id: call_id.to_string(),
            name: name.to_string(),
            outcome: outcome.into(),
        })
        .await;
    }

    async fn retract_segment(&self) {
        self.emit(AiStreamEvent::SegmentRetracted).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::models::AiToolOutcome;

    #[tokio::test]
    async fn sink_maps_each_progress_call_to_its_event() {
        let (tx, mut rx) = mpsc::channel(16);
        let sink = WebStreamSink::new(tx, CancellationToken::new());

        sink.report_stage("thinking").await;
        sink.report_tool_call("add_todo").await;
        sink.report_conversation("conv-1").await;
        sink.push_delta("Hel").await;
        sink.retract_segment().await;
        sink.flush().await;
        sink.report_tool_started("call_0", "add_todo").await;
        sink.report_tool_finished("call_0", "add_todo", ToolStepOutcome::Failed)
            .await;
        drop(sink);

        let mut got = Vec::new();
        while let Some(e) = rx.recv().await {
            got.push(e);
        }
        assert_eq!(
            got,
            vec![
                AiStreamEvent::Started {
                    conversation_id: "conv-1".into()
                },
                AiStreamEvent::Delta { text: "Hel".into() },
                AiStreamEvent::SegmentRetracted,
                AiStreamEvent::SegmentEnd,
                AiStreamEvent::ToolStarted {
                    call_id: "call_0".into(),
                    name: "add_todo".into()
                },
                AiStreamEvent::ToolFinished {
                    call_id: "call_0".into(),
                    name: "add_todo".into(),
                    outcome: AiToolOutcome::Failed
                },
            ]
        );
    }

    #[tokio::test]
    async fn a_dropped_receiver_cancels_the_token() {
        let (tx, rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let sink = WebStreamSink::new(tx, cancel.clone());
        drop(rx);

        sink.push_delta("x").await;

        assert!(cancel.is_cancelled());
    }
}
