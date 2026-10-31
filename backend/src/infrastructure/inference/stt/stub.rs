//! A fake OpenAI-compatible transcription server for tests of the code that
//! calls `SttClient`, so they never contact a real provider.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::{http::StatusCode, routing::post, Router};

use super::SttClient;
use crate::infrastructure::inference::test_support::spawn_server;

/// Counts the requests a stub server has answered.
#[derive(Clone, Default)]
pub(crate) struct CallCount(Arc<AtomicUsize>);

impl CallCount {
    pub(crate) fn get(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

/// A server answering every transcription with `text`; returns the client
/// pointed at it and the request counter.
pub(crate) async fn stub_stt(text: &'static str) -> (SttClient, CallCount) {
    let calls = CallCount::default();
    let seen = calls.clone();
    let app = Router::new().route(
        "/v1/audio/transcriptions",
        post(move || {
            let seen = seen.clone();
            async move {
                seen.0.fetch_add(1, Ordering::SeqCst);
                (
                    StatusCode::OK,
                    [("content-type", "application/json")],
                    format!(r#"{{"text":"{text}"}}"#),
                )
            }
        }),
    );
    let base = format!("{}/v1", spawn_server(app).await);
    (SttClient::new(Some(base), None, "stub".to_string()), calls)
}
