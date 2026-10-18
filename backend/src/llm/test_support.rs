//! Scripted-LLM helpers shared by the turn tests and the chat front door's
//! tests. A local axum server stands in for the provider; nothing here
//! contacts an external service.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};

use crate::application::AppServices;
use crate::db::DbPool;
use crate::domain::accounts::AccountId;
use crate::infrastructure::inference::test_support::spawn_server;

/// Every request body the scripted endpoint received, in order.
pub type Seen = Arc<Mutex<Vec<serde_json::Value>>>;

pub fn text_reply(text: &str) -> serde_json::Value {
    serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
        "message":{"role":"assistant","content":text},"finish_reason":"stop"}]})
}

/// One assistant message that asks for several tool calls in the same round.
pub fn tool_calls_reply(calls: &[(&str, serde_json::Value)]) -> serde_json::Value {
    let calls: Vec<serde_json::Value> = calls
        .iter()
        .enumerate()
        .map(|(i, (name, args))| {
            serde_json::json!({"id": format!("call_{i}"), "type": "function",
                "function": {"name": name, "arguments": args.to_string()}})
        })
        .collect();
    serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
        "message":{"role":"assistant","content":null,"tool_calls":calls},
        "finish_reason":"tool_calls"}]})
}

pub fn tool_reply(name: &str, args: serde_json::Value) -> serde_json::Value {
    tool_calls_reply(&[(name, args)])
}

/// Serves the scripted replies in order (then "done") as plain JSON
/// completions and records every request body it receives.
pub async fn scripted_replies(replies: Vec<serde_json::Value>) -> (String, Seen) {
    type Queue = Arc<Mutex<VecDeque<serde_json::Value>>>;
    let seen: Seen = Arc::default();
    let queue: Queue = Arc::new(Mutex::new(replies.into()));
    let app = Router::new()
        .route(
            "/chat/completions",
            post(
                |State((queue, seen)): State<(Queue, Seen)>,
                 Json(body): Json<serde_json::Value>| async move {
                    seen.lock().unwrap().push(body);
                    let next = queue.lock().unwrap().pop_front();
                    Json(next.unwrap_or_else(|| text_reply("done")))
                },
            ),
        )
        .with_state((queue, seen.clone()));
    (spawn_server(app).await, seen)
}

/// Like `scripted_replies`, but answers as the SSE stream a turn with a
/// progress sink asks for: each scripted message becomes one delta chunk.
pub async fn scripted_stream_replies(replies: Vec<serde_json::Value>) -> String {
    type Queue = Arc<Mutex<VecDeque<serde_json::Value>>>;
    let queue: Queue = Arc::new(Mutex::new(replies.into()));
    let app = Router::new()
        .route(
            "/chat/completions",
            post(|State(queue): State<Queue>| async move {
                let next = queue
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| text_reply("done"));
                let mut delta = next["choices"][0]["message"].clone();
                if let Some(calls) = delta["tool_calls"].as_array_mut() {
                    for (i, call) in calls.iter_mut().enumerate() {
                        call["index"] = serde_json::json!(i);
                    }
                }
                let chunk = serde_json::json!({"id":"c","model":"m","choices":[{"index":0,
                    "delta":delta,"finish_reason":null}]});
                (
                    [("content-type", "text/event-stream")],
                    format!("data: {chunk}\n\ndata: [DONE]\n\n"),
                )
            }),
        )
        .with_state(queue);
    spawn_server(app).await
}

pub fn test_config(base_url: String) -> crate::config::AiEnvConfig {
    crate::config::AiEnvConfig {
        base_url: Some(base_url),
        api_key: Some("sk-test".to_string()),
        allow_private_base_url: true,
        ..Default::default()
    }
}

/// A migrated pool, an account and one todo it owns.
pub async fn pool_with_todo(seed: &str) -> (DbPool, AccountId, i64) {
    let pool = crate::db::pool::test_pool().await;
    crate::db::migrate::run_migrations_unlocked(&pool).await;
    let owner = AccountId::from_seed(seed);
    crate::db::pool::seed_account(&pool, &owner).await;
    let todo = crate::domain::todos::NewTodo {
        room_id: "!room:test".to_string(),
        sender_name: "tester".to_string(),
        text: "Pay rent".to_string(),
        priority: crate::domain::todos::TodoPriority::Medium,
        due_at: None,
        remind_before_minutes: None,
        remind_room_id: None,
    };
    let (item, _) = AppServices::new(&pool)
        .todos
        .create(&owner, todo)
        .await
        .unwrap();
    (pool, owner, item.id)
}

/// The content of every `tool` message in a recorded request.
pub fn tool_message_contents(request: &serde_json::Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "tool")
        .map(|m| m["content"].as_str().unwrap_or_default().to_string())
        .collect()
}
