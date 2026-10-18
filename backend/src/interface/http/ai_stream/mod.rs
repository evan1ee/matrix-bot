//! Streaming web AI chat: `POST /api/v1/ai/chat/stream`, the event type sent
//! to the browser, the sink that turns a turn's progress into those events,
//! and the per-account cap on running turns.
//!
//! The turn runs on its own task, never inside the response body: dropping a
//! body future would cancel `run_turn` at any `.await`, including halfway
//! through a tool's writes. The body instead holds a drop guard of the turn's
//! token, so a client that leaves (Stop, closed tab) cancels the turn, and
//! `run_turn` stops at its next safe point.

mod events;
mod sink;
mod turns;

use std::convert::Infallible;
use std::time::Duration;

use axum::extract::State;
use axum::http::{header, HeaderValue};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures_util::stream::{self, Stream};
use tokio::sync::mpsc;
use tokio_util::sync::{CancellationToken, DropGuard};
use tracing::{error, warn};

pub(crate) use events::{error_event, AiStreamEvent};
pub(crate) use sink::WebStreamSink;
pub(crate) use turns::{AiTurns, TurnPermit, MAX_TURNS_PER_ACCOUNT};

use crate::domain::accounts::AccountId;
use crate::interface::http::ai::{validate_prompt, TURN_TIMED_OUT, TURN_TIME_LIMIT};
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;
use crate::llm::error::AiError;
use crate::llm::models::AiChatRequest;
use crate::llm::{run_turn, AiTurnResult, LlmTurnContext};

/// Events buffered between the turn and the response body. A full channel
/// makes the sink wait (backpressure); nothing is dropped.
const EVENT_BUFFER: usize = 64;
/// How long a timed-out turn gets, after its token is cancelled, to reach a
/// safe point and store its stopped marker itself.
const STOP_GRACE: Duration = Duration::from_secs(5);
const KEEP_ALIVE: Duration = Duration::from_secs(15);

/// Everything a turn task owns; nothing borrowed from the request.
struct TurnJob {
    state: ApiState,
    owner: AccountId,
    room_id: String,
    prompt: String,
    conversation_id: Option<String>,
    token: CancellationToken,
    tx: mpsc::Sender<AiStreamEvent>,
}

#[utoipa::path(
    post,
    tag = "ai",
    path = "/api/v1/ai/chat/stream",
    request_body = AiChatRequest,
    responses(
        (status = 200, description = "One JSON `AiStreamEvent` per SSE `data:` line, `started` first, then `done` or `error` last", content_type = "text/event-stream", body = AiStreamEvent),
        (status = 400, description = "Empty or too-long prompt", body = ApiErrorResponse),
        (status = 404, description = "Unknown or foreign conversation", body = ApiErrorResponse),
        (status = 429, description = "The account already runs its maximum of streaming turns", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn chat_stream(
    State(state): State<ApiState>,
    user: AuthUser,
    Json(payload): Json<AiChatRequest>,
) -> Result<Response, ApiError> {
    let prompt = validate_prompt(&payload.prompt)?.to_string();
    let owner = user.account_id;
    if let Some(id) = payload.conversation_id.as_deref() {
        state
            .services
            .llm
            .get_conversation(id, &owner)
            .await?
            .ok_or_else(|| ApiError::not_found("Conversation not found"))?;
    }
    let permit = state.ai_turns.try_acquire(&owner).ok_or_else(|| {
        ApiError::too_many_requests("Too many AI replies running; wait for one to finish")
    })?;
    let room_id = state
        .services
        .rooms
        .resolve_target_room(&owner, payload.room_id.as_deref())
        .await;

    let token = state.shutdown.child_token();
    let (tx, rx) = mpsc::channel(EVENT_BUFFER);
    let tracker = state.ai_turns.tracker().clone();
    let job = TurnJob {
        state,
        owner,
        room_id,
        prompt,
        conversation_id: payload.conversation_id,
        token: token.clone(),
        tx,
    };
    tracker.spawn(run_job(job, permit));

    let mut response = Sse::new(event_stream(rx, token.drop_guard()))
        .keep_alive(KeepAlive::new().interval(KEEP_ALIVE))
        .into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    Ok(response)
}

/// The response body: every event the turn sends, ending when the turn task
/// drops its senders. Owns `guard`, so dropping the body cancels the turn.
fn event_stream(
    rx: mpsc::Receiver<AiStreamEvent>,
    guard: DropGuard,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream::unfold((rx, guard), |(mut rx, guard)| async move {
        let event = rx.recv().await?;
        let frame = Event::default().json_data(&event).unwrap_or_else(|err| {
            // Unreachable for these plain types; a broken frame beats a
            // silently missing one.
            error!("Failed to serialize an AI stream event: {err}");
            Event::default().comment("unserializable event")
        });
        Some((Ok(frame), (rx, guard)))
    })
}

/// Runs one turn and sends its last event. `permit` is moved in so the slot
/// is released however the task ends, including a panic.
async fn run_job(job: TurnJob, permit: TurnPermit) {
    let last = match run_with_limit(&job).await {
        None => Some(AiStreamEvent::Error {
            status: 504,
            message: TURN_TIMED_OUT.to_string(),
        }),
        Some(Ok(result)) => Some(done_event(result)),
        // The client left or the server is stopping: nobody is listening,
        // and `run_turn` already stored the stopped marker.
        Some(Err(AiError::Cancelled)) => None,
        Some(Err(err)) => {
            error!("AI stream turn failed for {}: {}", job.owner, err);
            Some(error_event(&err))
        }
    };
    // Free the slot before the last event, so a client that starts its next
    // turn on reading `done` is never refused by this one.
    drop(permit);
    if let Some(event) = last {
        // A send error means the client left; there is no one to tell.
        let _ = job.tx.send(event).await;
    }
}

/// `run_turn` under the 180 s limit; `None` when it ran out. On timeout the
/// turn is cancelled rather than dropped, and gets [`STOP_GRACE`] to reach a
/// safe point and store its stopped marker; past that it is dropped (it never
/// cancels mid tool round, so a long tool can outlast the grace).
async fn run_with_limit(job: &TurnJob) -> Option<Result<AiTurnResult, AiError>> {
    let sink = WebStreamSink::new(job.tx.clone(), job.token.clone());
    let ctx = LlmTurnContext::new(
        &job.state.services,
        &job.state.db.pool,
        &job.state.ai,
        &job.owner,
        &job.room_id,
        Some(&sink),
    )
    .with_cancellation(job.token.clone());
    let turn = run_turn(&ctx, &job.prompt, job.conversation_id.as_deref());
    tokio::pin!(turn);

    match tokio::time::timeout(TURN_TIME_LIMIT, &mut turn).await {
        Ok(result) => Some(result),
        Err(_) => {
            warn!("AI stream turn timed out after 180s for {}", job.owner);
            job.token.cancel();
            if tokio::time::timeout(STOP_GRACE, &mut turn).await.is_err() {
                warn!(
                    "AI stream turn for {} did not stop within {:?} of its timeout",
                    job.owner, STOP_GRACE
                );
            }
            None
        }
    }
}

fn done_event(result: AiTurnResult) -> AiStreamEvent {
    AiStreamEvent::Done {
        conversation_id: result.conversation_id,
        message: result.message,
        model_used: result.model_used,
        tool_calls_executed: u32::try_from(result.tool_calls_executed.len()).unwrap_or(u32::MAX),
    }
}

#[cfg(test)]
mod tests;
