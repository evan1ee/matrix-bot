//! Streaming execution helper for AI turns with in-place Matrix message updates.

use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;

use crate::bot::stage::StageReporter;
use crate::modules::ai::client::{AiClient, ResolvedAiConfig, StreamTurnResult};
use crate::modules::ai::models::{ChatCompletionRequest, ToolCall};

/// Executes an AI turn with real-time streaming updates to Matrix if a `StageReporter`
/// is available, falling back cleanly to non-streaming on any transport error.
pub async fn execute_stream_or_complete(
    client: &AiClient,
    config: &ResolvedAiConfig,
    request: &ChatCompletionRequest,
    stage_reporter: Option<&StageReporter>,
) -> Result<(String, Vec<ToolCall>), String> {
    if let Some(reporter) = stage_reporter {
        let throttler = Arc::new(Mutex::new(reporter.throttler()));
        let throttler_clone = throttler.clone();
        match client
            .chat_completion_stream(config, request, move |delta| {
                let t = throttler_clone.clone();
                async move {
                    t.lock().await.push_delta(&delta).await;
                }
            })
            .await
        {
            Ok(StreamTurnResult { content, tool_calls }) => Ok((content, tool_calls)),
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
    client: &AiClient,
    config: &ResolvedAiConfig,
    request: &ChatCompletionRequest,
) -> Result<(String, Vec<ToolCall>), String> {
    let resp = client
        .chat_completion(config, request)
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
