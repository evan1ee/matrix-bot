use axum::{
    extract::State,
    response::sse::{Event, Sse},
};
use futures_util::stream::StreamExt;
use std::convert::Infallible;
use std::time::Duration;

use crate::interface::http::auth::{ApiState, AuthUser};
use crate::interface::http::error::ApiError;

/// SSE endpoint streaming real-time system events to authenticated web clients.
///
/// Emits unnamed event-stream messages whose body carries `type` and `payload`.
///
/// The broadcast channel is process-wide, so every subscriber sees every event.
/// This handler is what narrows it back down: a client receives only events it
/// owns, matching the per-owner scoping the REST endpoints enforce.
// This text is the frozen OpenAPI description (byte-identical contract): the
// hub now scopes per account structurally, but reword it only with the
// contract, not with the code.
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
    // Ends when the server shuts down: an SSE response never finishes on its
    // own, so without this every open tab would hold graceful shutdown open
    // until the composition root's deadline aborted the server.
    let stream = state
        .hub
        .subscribe(user.account_id)
        .take_until(state.shutdown.clone().cancelled_owned())
        .map(|line| Ok(Event::default().data(&*line)));

    Ok(Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::accounts::{AccountId, Credential};

    #[tokio::test]
    async fn test_sse_handler_creation() {
        let pool = crate::db::pool::test_pool().await;
        let state = ApiState::new(
            crate::db::Database { pool },
            None,
            "https://matrix.org".to_string(),
            reqwest::Client::new(),
        );
        let user = AuthUser::for_seed("acct_alice");

        let _sse = sse_handler(State(state), user).await.unwrap();
    }

    fn test_state(pool: crate::db::DbPool) -> ApiState {
        ApiState::new(
            crate::db::Database { pool },
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
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;
        crate::db::pool::seed_accounts(&pool, &["acct_mine", "acct_alice"]).await;
        let state = test_state(pool.clone());

        let accounts = &state.services.accounts;
        let account = accounts.create("evan", "active").await.unwrap();
        let token = accounts
            .create_session(&account, None, None, None)
            .await
            .unwrap();

        // A live cookie resolves to the owning account.
        let principal = accounts
            .resolve_credential(&Credential::Cookie(&token))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(principal.account_id, account);

        // A revoked one does not, so the stream closes to it on reconnect.
        accounts
            .end_session(&principal.session_id_hash)
            .await
            .unwrap();
        assert!(accounts
            .resolve_credential(&Credential::Cookie(&token))
            .await
            .unwrap()
            .is_none());
    }

    /// The hub carries every account's lines; a subscriber must only ever see
    /// its own. Drives `sse_handler` itself, so registering a subscription
    /// under the wrong account fails this test.
    #[tokio::test]
    async fn test_stream_delivers_only_the_subscribers_own_events() {
        use axum::response::IntoResponse;
        use std::sync::Arc;

        let pool = crate::db::pool::test_pool().await;
        let state = test_state(pool);
        let shutdown = state.shutdown.clone();
        let hub = state.hub.clone();
        let subscriber = AuthUser::named_seed("evan", "acct_mine");
        let mine = subscriber.account_id;
        assert_eq!(mine, AccountId::from_seed("acct_mine"));
        let theirs = AccountId::from_seed("acct_theirs");

        let sse = sse_handler(State(state), subscriber).await.unwrap();

        assert_eq!(hub.push(&theirs, Arc::from("theirs")), 0);
        assert_eq!(hub.push(&mine, Arc::from("mine")), 1);

        let mut body = sse.into_response().into_body().into_data_stream();
        let first = tokio::time::timeout(Duration::from_secs(1), body.next())
            .await
            .expect("the subscriber's own line must arrive")
            .expect("the stream is open")
            .unwrap();
        assert!(String::from_utf8_lossy(&first).contains("mine"));

        // Ending the stream proves nothing else was queued for this account.
        shutdown.cancel();
        let mut rest = String::new();
        while let Some(chunk) = body.next().await {
            rest.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }
        assert!(
            !rest.contains("theirs"),
            "another account's line leaked: {rest}"
        );
    }

    /// An SSE response never finishes on its own; it must end when the server
    /// shuts down, or every open tab holds graceful shutdown open until the
    /// composition root's deadline aborts the server.
    #[tokio::test]
    async fn test_stream_ends_when_the_server_shuts_down() {
        use axum::response::IntoResponse;

        let pool = crate::db::pool::test_pool().await;
        let state = test_state(pool);
        let shutdown = state.shutdown.clone();
        // Held open as the running server holds it: dropping the last sender
        // would end the stream on its own and hide a missing shutdown hook.
        let _hub = state.hub.clone();
        let sse = sse_handler(State(state), AuthUser::for_seed("acct_alice"))
            .await
            .unwrap();

        shutdown.cancel();

        let body = sse.into_response().into_body();
        let drained = tokio::time::timeout(
            Duration::from_secs(1),
            axum::body::to_bytes(body, usize::MAX),
        )
        .await;
        assert!(
            matches!(drained, Ok(Ok(_))),
            "the SSE body must end once shutdown is cancelled"
        );
    }
}
