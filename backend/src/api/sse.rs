use axum::{
    extract::State,
    response::sse::{Event, Sse},
};
use futures_util::stream::StreamExt;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::auth::ApiState;

/// SSE endpoint streaming real-time system events to authenticated web clients.
///
/// Each event includes an `event:` type field (e.g. `todo_created`, `reminder_triggered`)
/// allowing frontends to selectively listen via `EventSource.addEventListener()`.
#[utoipa::path(
    get,
    path = "/api/v1/events",
    responses(
        (status = 200, description = "Server-Sent Events (SSE) live stream", content_type = "text/event-stream")
    ),
    security(("bearer_auth" = []))
)]
pub async fn sse_handler(
    State(state): State<ApiState>,
) -> Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.broadcaster.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|item| async move {
        match item {
            Ok(msg) => {
                let event_type = msg.event_type();
                let json_str = serde_json::to_string(&msg).ok()?;
                Some(Ok(Event::default().event(event_type).data(json_str)))
            }
            Err(_) => None,
        }
    });

    Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_sse_handler_creation() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );

        let _sse = sse_handler(State(state)).await;
    }
}
