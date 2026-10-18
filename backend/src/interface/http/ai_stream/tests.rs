use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use futures_util::StreamExt;

use super::*;
use crate::db::Database;
use crate::domain::accounts::AccountId;
use crate::interface::http::auth::{ApiState, AuthUser};
use crate::llm::models::AiChatRequest;
use crate::llm::test_support::{
    hanging_stream_llm, scripted_stream_replies, test_config, text_reply,
};

/// A migrated pool, an account, and API state whose AI provider is `llm`.
async fn setup_state(llm: String) -> (ApiState, AccountId) {
    let pool = crate::db::pool::test_pool_sized(6).await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let owner = AccountId::from_seed("stream-owner");
    crate::db::pool::seed_account(&pool, &owner).await;
    let mut state = ApiState::new(
        Database { pool },
        None,
        "https://matrix.org".to_string(),
        reqwest::Client::new(),
    );
    state.ai = test_config(llm);
    (state, owner)
}

fn request(prompt: &str, conversation_id: Option<String>) -> Json<AiChatRequest> {
    Json(AiChatRequest {
        room_id: None,
        conversation_id,
        prompt: prompt.to_string(),
    })
}

async fn open(state: &ApiState, owner: AccountId, prompt: &str) -> Result<Response, ApiError> {
    chat_stream(
        State(state.clone()),
        AuthUser::for_account(owner),
        request(prompt, None),
    )
    .await
}

/// The events in an SSE body text, in order (keep-alive comments skipped).
fn parse_events(text: &str) -> Vec<serde_json::Value> {
    text.lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|data| serde_json::from_str(data.trim()).expect("each data line is JSON"))
        .collect()
}

/// Reads the whole body; a finished turn ends it.
async fn all_events(response: Response) -> Vec<serde_json::Value> {
    let bytes = tokio::time::timeout(
        Duration::from_secs(10),
        axum::body::to_bytes(response.into_body(), usize::MAX),
    )
    .await
    .expect("the stream ends once the turn ends")
    .expect("the body reads cleanly");
    parse_events(&String::from_utf8_lossy(&bytes))
}

/// Reads until the first event arrives, leaving the rest of the body open.
async fn first_event(body: &mut axum::body::BodyDataStream) -> serde_json::Value {
    let mut text = String::new();
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(5), body.next())
            .await
            .expect("an event within 5 s")
            .expect("the stream is still open")
            .unwrap();
        text.push_str(&String::from_utf8_lossy(&chunk));
        if let Some(event) = parse_events(&text).into_iter().next() {
            return event;
        }
    }
}

/// Polls until the conversation's last stored message ends with the stopped
/// marker, or 2 s pass.
async fn assert_stopped_marker_stored(state: &ApiState, conversation_id: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let messages = state
            .services
            .llm
            .get_recent_messages(conversation_id, 20)
            .await
            .unwrap();
        if let Some(last) = messages.last() {
            if last.role == "assistant" && last.content.ends_with("_(stopped)_") {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no stopped marker within 2 s; stored: {:?}",
            messages
                .iter()
                .map(|m| (&m.role, &m.content))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn stream_sends_started_deltas_and_done_in_order() {
    let llm = scripted_stream_replies(vec![text_reply("Hello there")]).await;
    let (state, owner) = setup_state(llm).await;

    let response = open(&state, owner, "hi").await.expect("the stream opens");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(headers["content-type"], "text/event-stream");
    assert_eq!(headers["cache-control"], "no-cache");
    assert_eq!(headers["x-accel-buffering"], "no");

    let events = all_events(response).await;
    let types: Vec<&str> = events.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types, vec!["started", "delta", "segment_end", "done"]);
    let conversation_id = events[0]["conversation_id"].as_str().unwrap();
    assert_eq!(events[1]["text"], "Hello there");
    let done = &events[3];
    assert_eq!(done["conversation_id"], conversation_id);
    assert_eq!(done["message"], "Hello there");
    assert_eq!(done["tool_calls_executed"], 0);
}

#[tokio::test]
async fn a_foreign_conversation_is_404_before_streaming() {
    let llm = scripted_stream_replies(vec![]).await;
    let (state, owner) = setup_state(llm).await;
    let other = AccountId::from_seed("stream-other");
    crate::db::pool::seed_account(&state.db.pool, &other).await;
    let theirs = state
        .services
        .llm
        .get_or_create_conversation(&other, "!room:test", 0)
        .await
        .unwrap();

    for id in [theirs.id.clone(), "no-such-conversation".to_string()] {
        let err = chat_stream(
            State(state.clone()),
            AuthUser::for_account(owner),
            request("hi", Some(id)),
        )
        .await
        .unwrap_err();
        assert_eq!(err.status, StatusCode::NOT_FOUND);
        assert_eq!(err.message, "Conversation not found");
    }
}

#[tokio::test]
async fn an_empty_prompt_is_a_json_400() {
    let llm = scripted_stream_replies(vec![]).await;
    let (state, owner) = setup_state(llm).await;

    let err = open(&state, owner, "   ").await.unwrap_err();

    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert_eq!(err.message, "Prompt cannot be empty");
}

#[tokio::test]
async fn a_provider_failure_after_open_is_an_error_event_with_503() {
    let (state, owner) = setup_state("http://127.0.0.1:1".to_string()).await;

    let response = open(&state, owner, "hi").await.expect("the stream opens");
    let events = all_events(response).await;

    assert_eq!(events.first().unwrap()["type"], "started");
    let last = events.last().unwrap();
    assert_eq!(last["type"], "error", "{events:?}");
    assert_eq!(last["status"], 503);
}

#[tokio::test]
async fn a_third_concurrent_turn_is_429() {
    let (state, owner) = setup_state(hanging_stream_llm("Hel").await).await;

    let first = open(&state, owner, "one").await.expect("first opens");
    let second = open(&state, owner, "two").await.expect("second opens");
    let err = open(&state, owner, "three").await.unwrap_err();

    assert_eq!(err.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        err.message,
        "Too many AI replies running; wait for one to finish"
    );
    drop((first, second));
    state.shutdown.cancel();
}

#[tokio::test]
async fn turn_permit_is_released_when_the_turn_errors() {
    let (failing, owner) = setup_state("http://127.0.0.1:1".to_string()).await;
    for _ in 0..2 {
        let events = all_events(open(&failing, owner, "hi").await.unwrap()).await;
        assert_eq!(events.last().unwrap()["type"], "error");
    }

    // Same counts (the clone shares them), a provider that answers.
    let mut working = failing.clone();
    working.ai = test_config(scripted_stream_replies(vec![text_reply("ok")]).await);
    let events = all_events(open(&working, owner, "hi").await.expect("accepted")).await;

    assert_eq!(events.last().unwrap()["type"], "done");
}

#[tokio::test]
async fn dropping_the_response_body_cancels_the_turn() {
    let (state, owner) = setup_state(hanging_stream_llm("Hel").await).await;
    let response = open(&state, owner, "hi").await.unwrap();
    let mut body = response.into_body().into_data_stream();

    let started = first_event(&mut body).await;
    assert_eq!(started["type"], "started");
    drop(body);

    assert_stopped_marker_stored(&state, started["conversation_id"].as_str().unwrap()).await;
}

#[tokio::test]
async fn shutdown_cancels_a_running_turn() {
    let (state, owner) = setup_state(hanging_stream_llm("Hel").await).await;
    let response = open(&state, owner, "hi").await.unwrap();
    let mut body = response.into_body().into_data_stream();
    let started = first_event(&mut body).await;

    state.shutdown.cancel();

    assert_stopped_marker_stored(&state, started["conversation_id"].as_str().unwrap()).await;
    let tracker = state.ai_turns.tracker();
    tracker.close();
    tokio::time::timeout(Duration::from_secs(2), tracker.wait())
        .await
        .expect("every turn task ends within 2 s of shutdown");
}
