//! What a stopped turn leaves behind in the conversation.

use tracing::{info, warn};

use super::context::LlmTurnContext;
use super::error::AiError;

const STOPPED_MARKER: &str = "_(stopped)_";

/// The assistant message stored for a turn the user stopped: the text of the
/// segment that was arriving, if any, then the marker.
pub fn stopped_message(partial: &str) -> String {
    if partial.trim().is_empty() {
        STOPPED_MARKER.to_string()
    } else {
        format!("{}\n\n{STOPPED_MARKER}", partial.trim_end())
    }
}

/// True when the caller has stopped the turn.
pub fn is_cancelled(ctx: &LlmTurnContext<'_>) -> bool {
    ctx.cancel.as_ref().is_some_and(|t| t.is_cancelled())
}

/// Ends a stopped turn: stores the stopped message through the service (so
/// `AiConversationUpdated` fires) and returns the error `run_turn` reports.
/// A stop is the caller's choice, not a failure, so it is logged at `info`.
pub async fn finish_stopped(
    ctx: &LlmTurnContext<'_>,
    conversation_id: &str,
    partial: &str,
) -> AiError {
    info!(
        "LLM turn stopped by the caller for owner {} in room {}",
        ctx.owner_id, ctx.room_id
    );
    if let Err(e) = ctx
        .services
        .llm
        .append_assistant_message(ctx.owner_id, conversation_id, &stopped_message(partial))
        .await
    {
        warn!("Failed to persist the stopped marker: {}", e);
    }
    AiError::Cancelled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_stopped_before_any_text_stores_only_the_marker() {
        assert_eq!(stopped_message(""), "_(stopped)_");
        assert_eq!(stopped_message("  \n"), "_(stopped)_");
    }

    #[test]
    fn a_turn_stopped_mid_reply_keeps_the_partial_text() {
        assert_eq!(stopped_message("Half"), "Half\n\n_(stopped)_");
        assert_eq!(stopped_message("Half \n"), "Half\n\n_(stopped)_");
    }
}
