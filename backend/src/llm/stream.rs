//! Streaming execution helper for AI turns that report text as it arrives.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::progress::ProgressSink;
use super::resolved_config::ResolvedAiConfig;
use crate::infrastructure::inference::llm::{
    ChatCompletionRequest, LlmClient, StreamedCompletion, ToolCall,
};

/// Why one LLM call produced no reply.
#[derive(Debug)]
pub enum StreamError {
    /// The provider failed, both streaming and on the fallback.
    Failed(String),
    /// The caller stopped the turn; `partial` is the text of the current
    /// segment that reached the sink before the stop.
    Cancelled { partial: String },
}

/// Executes an AI turn with real-time streaming updates to the `ProgressSink`, if one
/// is given, falling back cleanly to non-streaming on any transport error. Cancelling
/// `cancel` drops the in-flight request and returns what had arrived so far.
pub async fn execute_stream_or_complete(
    client: &LlmClient,
    config: &ResolvedAiConfig,
    request: &ChatCompletionRequest,
    progress: Option<&dyn ProgressSink>,
    cancel: Option<&CancellationToken>,
) -> Result<(String, Vec<ToolCall>), StreamError> {
    let Some(sink) = progress else {
        return until_cancelled(cancel, fallback_complete(client, config, request))
            .await
            .ok_or_else(|| StreamError::Cancelled {
                partial: String::new(),
            })?;
    };
    // Whether any delta reached the sink (a failed stream must then retract
    // it) and the text shown so far (a stop keeps it).
    let streamed = AtomicBool::new(false);
    let shown = Mutex::new(String::new());
    let (streamed_ref, shown_ref) = (&streamed, &shown);
    let stream = client.chat_completion_stream(config.endpoint(), request, move |delta| {
        streamed_ref.store(true, Ordering::Relaxed);
        if let Ok(mut text) = shown_ref.lock() {
            text.push_str(&delta);
        }
        async move { sink.push_delta(&delta).await }
    });
    let Some(outcome) = until_cancelled(cancel, stream).await else {
        let partial = shown.lock().map(|t| t.clone()).unwrap_or_default();
        return Err(StreamError::Cancelled { partial });
    };
    match outcome {
        Ok(StreamedCompletion {
            content,
            tool_calls,
        }) => {
            sink.flush().await;
            Ok((content, tool_calls))
        }
        Err(err) => {
            warn!("AI stream error ({:?}), falling back to non-streaming", err);
            // Text already shown came from a stream that failed; the fallback
            // re-answers in full, so the partial text must not stay.
            if streamed.load(Ordering::Relaxed) {
                sink.retract_segment().await;
            }
            let (content, tool_calls) =
                until_cancelled(cancel, fallback_complete(client, config, request))
                    .await
                    .ok_or_else(|| StreamError::Cancelled {
                        partial: String::new(),
                    })??;
            if !content.is_empty() {
                sink.push_delta(&content).await;
                sink.flush().await;
            }
            Ok((content, tool_calls))
        }
    }
}

/// Runs `work` to completion, or returns `None` as soon as `cancel` fires
/// (dropping `work`, and with it any request in flight).
pub async fn until_cancelled<T>(
    cancel: Option<&CancellationToken>,
    work: impl Future<Output = T>,
) -> Option<T> {
    match cancel {
        Some(token) => tokio::select! {
            biased;
            _ = token.cancelled() => None,
            out = work => Some(out),
        },
        None => Some(work.await),
    }
}

async fn fallback_complete(
    client: &LlmClient,
    config: &ResolvedAiConfig,
    request: &ChatCompletionRequest,
) -> Result<(String, Vec<ToolCall>), StreamError> {
    let resp = client
        .chat_completion(config.endpoint(), request)
        .await
        .map_err(|e| StreamError::Failed(e.to_string()))?;

    let choice = resp.choices.into_iter().next();
    let msg = choice.map(|c| c.message);
    let content = msg
        .as_ref()
        .and_then(|m| m.content.clone())
        .unwrap_or_default();
    let tool_calls = msg.and_then(|m| m.tool_calls).unwrap_or_default();
    Ok((content, tool_calls))
}
