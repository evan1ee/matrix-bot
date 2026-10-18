//! The JSON objects carried by the AI chat stream, one per SSE `data:` line.

use serde::Serialize;
use utoipa::ToSchema;

use crate::interface::http::error::ApiError;
use crate::llm::error::AiError;
use crate::llm::models::AiToolOutcome;

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiStreamEvent {
    /// The conversation this turn writes to. Always first.
    Started {
        conversation_id: String,
    },
    /// Appended text for the current text segment.
    Delta {
        text: String,
    },
    /// The current text segment is finished.
    SegmentEnd,
    /// Throw away the current text segment: the open one if text is still
    /// arriving, otherwise the one that just ended.
    SegmentRetracted,
    ToolStarted {
        call_id: String,
        name: String,
    },
    ToolFinished {
        call_id: String,
        name: String,
        outcome: AiToolOutcome,
    },
    /// Final reply; always last on success.
    Done {
        conversation_id: String,
        message: String,
        model_used: String,
        tool_calls_executed: u32,
    },
    /// The turn failed after the stream opened; last event.
    Error {
        status: u16,
        message: String,
    },
}

/// The same status and client-safe text `/ai/chat` answers with for `err`.
pub fn error_event(err: &AiError) -> AiStreamEvent {
    let api = ApiError::from(err.clone());
    AiStreamEvent::Error {
        status: api.status.as_u16(),
        message: api.message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_snake_case_type() {
        let finished = AiStreamEvent::ToolFinished {
            call_id: "call_0".into(),
            name: "delete_todo".into(),
            outcome: AiToolOutcome::NeedsConfirmation,
        };
        let json = serde_json::to_value(&finished).unwrap();
        assert_eq!(json["type"], "tool_finished");
        assert_eq!(json["outcome"], "needs_confirmation");
        assert_eq!(
            serde_json::to_string(&AiStreamEvent::SegmentEnd).unwrap(),
            r#"{"type":"segment_end"}"#
        );
    }

    #[test]
    fn error_event_matches_api_error() {
        let provider = error_event(&AiError::Provider("down".into()));
        assert_eq!(
            provider,
            AiStreamEvent::Error {
                status: 503,
                message: "down".into()
            }
        );
        match error_event(&AiError::Timeout) {
            AiStreamEvent::Error { status, message } => {
                assert_eq!(status, 504);
                assert_eq!(message, "The AI provider did not respond in time");
            }
            other => panic!("unexpected event {other:?}"),
        }
    }
}
