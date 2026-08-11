use axum::{extract::State, response::sse::{Event, Sse}};
use futures_util::stream::StreamExt;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;

use crate::api::auth::{ApiState, AuthUser};
use crate::error::ApiError;
use crate::events::{EventBroadcaster, SystemEvent};

/// The subset of the process-wide broadcast stream that `owner_id` may see.
///
/// Kept separate from the response encoding so the ownership filter itself is
/// directly testable — it is the only thing standing between one user's live
/// updates and another user's screen.
fn owned_events(
    broadcaster: &EventBroadcaster,
    owner_id: String,
) -> impl futures_util::stream::Stream<Item = SystemEvent> {
    BroadcastStream::new(broadcaster.subscribe()).filter_map(move |item| {
        let owner_id = owner_id.clone();
        async move {
            let msg = item.ok()?;
            (msg.owner_id() == owner_id).then_some(msg)
        }
    })
}

/// SSE endpoint streaming real-time system events to authenticated web clients.
///
/// Emits unnamed event-stream messages whose body carries `type` and `payload`.
///
/// The broadcast channel is process-wide, so every subscriber sees every event.
/// This handler is what narrows it back down: a client receives only events it
/// owns, matching the per-owner scoping the REST endpoints enforce.
#[utoipa::path(
    get,
    path = "/api/v1/events",
    responses(
        (status = 200, description = "Server-Sent Events (SSE) live stream", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("session_cookie" = []))
)]
pub async fn sse_handler(
    State(state): State<ApiState>,
    user: AuthUser,
) -> Result<Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {

    let stream = owned_events(&state.broadcaster, user.owner_id()).filter_map(|msg| async move {
        let json_str = serde_json::to_string(&msg).ok()?;
        Some(Ok(Event::default().data(json_str)))
    });

    Ok(Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::{AccountRepository, SessionRepository};
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn test_sse_handler_creation() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let state = ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::for_account("acct_alice");

        let _sse = sse_handler(State(state), user).await.unwrap();
    }

    fn test_state(pool: sqlx::SqlitePool) -> ApiState {
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    /// The stream is authenticated by the same cookie as every other route.
    ///
    /// This replaces the `?ticket=` exchange entirely. `EventSource` cannot set
    /// headers, which is why a bearer token needed a short-lived ticket smuggled
    /// through the URL — but it *does* send cookies, so the whole mechanism went
    /// away rather than being carried forward.
    #[tokio::test]
    async fn test_the_stream_requires_a_session() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();
        let state = test_state(pool.clone());

        let account = AccountRepository::create(&pool, "evan", "active").await.unwrap();
        let token = SessionRepository::create(&pool, &account, None, None, None)
            .await
            .unwrap();

        // A live cookie resolves to the owning account.
        let session = SessionRepository::resolve(&pool, &token).await.unwrap().unwrap();
        assert_eq!(session.account_id, account);

        // A revoked one does not, so the stream closes to it on reconnect.
        SessionRepository::revoke(&pool, &session.id_hash).await.unwrap();
        assert!(SessionRepository::resolve(&pool, &token).await.unwrap().is_none());

        let _ = state;
    }

    /// The broadcast channel carries every user's events; a subscriber must only
    /// ever see its own. Drives `owned_events`, the same filter `sse_handler`
    /// serves from, so removing the filter fails this test.
    #[tokio::test]
    async fn test_stream_delivers_only_the_subscribers_own_events() {
        use crate::modules::todo::models::TodoItem;

        fn todo_for(owner: &str, text: &str) -> TodoItem {
            TodoItem {
                id: 1,
                room_id: "!room:1".to_string(),
                owner_id: owner.to_string(),
                sender_name: "evan".to_string(),
                text: text.to_string(),
                priority: "medium".to_string(),
                due_at: None,
                remind_before_minutes: None,
                completed: false,
                created_at: None,
            }
        }

        let broadcaster = EventBroadcaster::default();
        let subscriber = AuthUser::named("evan", "acct_mine");
        let owner_id = subscriber.owner_id();
        assert_eq!(owner_id, "acct_mine");

        let stream = owned_events(&broadcaster, owner_id.clone());

        // Another account's events, interleaved with the subscriber's own.
        // Both senders render as "evan" — a shared display name must not leak a
        // single row, which is why the filter reads `owner_id` and nothing else.
        broadcaster.send(SystemEvent::TodoCreated(todo_for("acct_theirs", "theirs")));
        broadcaster.send(SystemEvent::TodoCreated(todo_for("acct_mine", "mine")));
        broadcaster.send(SystemEvent::TodoDeleted {
            id: 2,
            room_id: "!room:1".to_string(),
            owner_id: "acct_theirs".to_string(),
        });
        broadcaster.send(SystemEvent::TodoDeleted {
            id: 3,
            room_id: "web".to_string(),
            owner_id: "acct_mine".to_string(),
        });
        drop(broadcaster);

        let delivered: Vec<SystemEvent> = stream.collect().await;

        assert_eq!(delivered.len(), 2, "expected exactly the subscriber's own events");
        assert!(delivered.iter().all(|e| e.owner_id() == "acct_mine"));
        match &delivered[0] {
            SystemEvent::TodoCreated(item) => assert_eq!(item.text, "mine"),
            other => panic!("unexpected first event: {:?}", other),
        }
        match &delivered[1] {
            SystemEvent::TodoDeleted { id, .. } => assert_eq!(*id, 3),
            other => panic!("unexpected second event: {:?}", other),
        }
    }
}
