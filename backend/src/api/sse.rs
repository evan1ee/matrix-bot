use axum::{
    extract::{Query, State},
    response::sse::{Event, Sse},
};
use futures_util::stream::StreamExt;
use jsonwebtoken::{decode, Validation};
use serde::Deserialize;
use std::convert::Infallible;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use utoipa::IntoParams;

use crate::api::auth::types::{Claims, JwtKeys, TokenScope};
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

#[derive(Deserialize, IntoParams)]
pub struct SseQuery {
    /// Short-lived ticket from `POST /api/v1/events/ticket`.
    pub ticket: Option<String>,
}

/// Resolves the subscriber, from a stream ticket or an `Authorization` header.
///
/// `EventSource` cannot set headers, so browsers pass a ticket; non-browser
/// clients can still authenticate normally.
async fn stream_subscriber(
    state: &ApiState,
    ticket: Option<String>,
    header_user: Option<AuthUser>,
) -> Result<AuthUser, ApiError> {
    if let Some(ticket) = ticket {
        let keys = JwtKeys::new(state.jwt_secret.as_bytes());
        let decoded = decode::<Claims>(&ticket, &keys.decoding, &Validation::default())
            .map_err(|_| ApiError::unauthorized("Invalid or expired stream ticket"))?;

        // An API token must not be usable here: that would re-open the door to
        // full-access credentials being passed through URLs.
        if decoded.claims.scope != TokenScope::SseStream {
            return Err(ApiError::unauthorized(
                "Query parameter must be a stream ticket, not an API token",
            ));
        }

        return Ok(AuthUser {
            subject: decoded.claims.sub,
            kind: decoded.claims.kind,
        });
    }

    header_user.ok_or_else(|| ApiError::unauthorized("Missing stream ticket or Authorization header"))
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
    params(SseQuery),
    responses(
        (status = 200, description = "Server-Sent Events (SSE) live stream", content_type = "text/event-stream"),
        (status = 401, description = "Unauthorized", body = ApiErrorResponse)
    ),
    security(("bearer_auth" = []))
)]
pub async fn sse_handler(
    State(state): State<ApiState>,
    Query(query): Query<SseQuery>,
    header_user: Option<AuthUser>,
) -> Result<Sse<impl futures_util::stream::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let user = stream_subscriber(&state, query.ticket, header_user).await?;

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
        let user = AuthUser::local("alice");

        let _sse = sse_handler(State(state), Query(SseQuery { ticket: None }), Some(user))
            .await
            .unwrap();
    }

    fn test_state(pool: sqlx::SqlitePool) -> ApiState {
        ApiState::new(
            crate::db::Database { pool },
            EventBroadcaster::default(),
            "secret".to_string(),
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        )
    }

    /// A stream ticket opens the stream; a full API token in the URL does not.
    ///
    /// That asymmetry is the entire point: it keeps long-lived credentials out
    /// of URLs, where proxies and browser history record them.
    #[tokio::test]
    async fn test_only_a_stream_ticket_is_accepted_from_the_query_string() {
        use crate::api::auth::matrix::{generate_jwt, generate_scoped_jwt};
        use crate::api::auth::types::AuthKind;

        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let state = test_state(pool);

        let ticket = generate_scoped_jwt(
            "@evan:mbm.one",
            AuthKind::Matrix,
            TokenScope::SseStream,
            chrono::Duration::seconds(30),
            &state.jwt_secret,
        )
        .unwrap();

        let subscriber = stream_subscriber(&state, Some(ticket), None).await.unwrap();
        assert_eq!(subscriber.owner_id(), "@evan:mbm.one");
        assert_eq!(subscriber.kind, AuthKind::Matrix);

        // A full-access API token presented the same way is refused.
        let api_token = generate_jwt("@evan:mbm.one", AuthKind::Matrix, &state.jwt_secret).unwrap();
        let err = stream_subscriber(&state, Some(api_token), None)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);

        // As is a forged one.
        let err = stream_subscriber(&state, Some("not-a-token".to_string()), None)
            .await
            .unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);

        // And no credential at all.
        let err = stream_subscriber(&state, None, None).await.unwrap_err();
        assert_eq!(err.status, axum::http::StatusCode::UNAUTHORIZED);
    }

    /// Non-browser clients can still use a normal header.
    #[tokio::test]
    async fn test_header_auth_still_opens_the_stream() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        let state = test_state(pool);

        let subscriber = stream_subscriber(&state, None, Some(AuthUser::local("alice")))
            .await
            .unwrap();
        assert_eq!(subscriber.owner_id(), "@web-alice:api");
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
                sender_mxid: owner.to_string(),
                sender_name: "evan".to_string(),
                text: text.to_string(),
                priority: "medium".to_string(),
                due_at: None,
                completed: false,
                created_at: None,
            }
        }

        let broadcaster = EventBroadcaster::default();
        let subscriber = AuthUser::local("evan");
        let owner_id = subscriber.owner_id();
        assert_eq!(owner_id, "@web-evan:api");

        let stream = owned_events(&broadcaster, owner_id.clone());

        // A Matrix user who shares the same display name, plus the subscriber.
        broadcaster.send(SystemEvent::TodoCreated(todo_for("@evan:mbm.one", "theirs")));
        broadcaster.send(SystemEvent::TodoCreated(todo_for("@web-evan:api", "mine")));
        broadcaster.send(SystemEvent::TodoDeleted {
            id: 2,
            room_id: "!room:1".to_string(),
            owner_id: "@evan:mbm.one".to_string(),
        });
        broadcaster.send(SystemEvent::TodoDeleted {
            id: 3,
            room_id: "web".to_string(),
            owner_id: "@web-evan:api".to_string(),
        });
        drop(broadcaster);

        let delivered: Vec<SystemEvent> = stream.collect().await;

        assert_eq!(delivered.len(), 2, "expected exactly the subscriber's own events");
        assert!(delivered.iter().all(|e| e.owner_id() == "@web-evan:api"));
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
