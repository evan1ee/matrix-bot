//! Streaming execution helper for AI turns that report text as it arrives.

use tracing::warn;

use super::progress::ProgressSink;
use super::resolved_config::ResolvedAiConfig;
use crate::infrastructure::inference::llm::{
    ChatCompletionRequest, LlmClient, StreamedCompletion, ToolCall,
};

/// Executes an AI turn with real-time streaming updates to the `ProgressSink`, if one
/// is given, falling back cleanly to non-streaming on any transport error.
pub async fn execute_stream_or_complete(
    client: &LlmClient,
    config: &ResolvedAiConfig,
    request: &ChatCompletionRequest,
    progress: Option<&dyn ProgressSink>,
) -> Result<(String, Vec<ToolCall>), String> {
    if let Some(sink) = progress {
        match client
            .chat_completion_stream(config.endpoint(), request, move |delta| async move {
                sink.push_delta(&delta).await;
            })
            .await
        {
            Ok(StreamedCompletion {
                content,
                tool_calls,
            }) => {
                sink.flush().await;
                Ok((content, tool_calls))
            }
            Err(err) => {
                warn!("AI stream error ({:?}), falling back to non-streaming", err);
                fallback_complete(client, config, request).await
            }
        }
    } else {
        fallback_complete(client, config, request).await
    }
}

async fn fallback_complete(
    client: &LlmClient,
    config: &ResolvedAiConfig,
    request: &ChatCompletionRequest,
) -> Result<(String, Vec<ToolCall>), String> {
    let resp = client
        .chat_completion(config.endpoint(), request)
        .await
        .map_err(|e| e.to_string())?;

    let choice = resp.choices.into_iter().next();
    let msg = choice.map(|c| c.message);
    let content = msg
        .as_ref()
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    let tool_calls = msg.and_then(|m| m.tool_calls).unwrap_or_default();
    Ok((content, tool_calls))
}
