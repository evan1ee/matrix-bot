//! `POST /api/webhooks/telegram`: Telegram's webhook delivery (T9, T12).
//!
//! Infrastructure for one caller, not application API, so it is not in OpenAPI
//! (documented in `docs/api.md`). It has no session: the secret Telegram echoes
//! in `X-Telegram-Bot-Api-Secret-Token` is the credential, so the cookie CSRF
//! rule (which lives in the `AuthUser` extractor) never applies. The route sees
//! only raw bytes and the adapter's `TelegramIngress`; parsing is the workers' job.

use std::net::SocketAddr;

use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::{HeaderMap, StatusCode},
};

use crate::interface::http::auth::recovery::extract_client_ip;
use crate::interface::http::auth::types::ApiState;
use crate::interface::http::rate_limit::check_telegram_webhook_limit;

const SECRET_HEADER: &str = "x-telegram-bot-api-secret-token";

/// A Telegram update is a few KiB at most; 1 MiB matches the server-wide request
/// limit (T9) and is enforced here too so the bound is local to the route and
/// testable without the server's layer stack.
const MAX_UPDATE_BYTES: usize = 1024 * 1024;

/// T9: rate limit, then the constant-time secret check (401, the body is not
/// read), then `submit`: 200 on accept, 503 on a full queue (Telegram redelivers
/// later). 404 when Telegram is not in webhook mode. Logs neither the header nor
/// the body.
pub async fn telegram_update(
    State(state): State<ApiState>,
    connect_info: Option<ConnectInfo<SocketAddr>>,
    headers: HeaderMap,
    body: Body,
) -> StatusCode {
    let Some(ingress) = state.telegram.as_ref() else {
        return StatusCode::NOT_FOUND;
    };
    let client_ip = extract_client_ip(connect_info.as_ref(), &headers, state.trust_forwarded_for);
    if !check_telegram_webhook_limit(&client_ip) {
        return StatusCode::TOO_MANY_REQUESTS;
    }
    let given = headers.get(SECRET_HEADER).map(|v| v.as_bytes());
    if !ingress.secret_matches(given) {
        return StatusCode::UNAUTHORIZED;
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_UPDATE_BYTES).await else {
        return StatusCode::PAYLOAD_TOO_LARGE;
    };
    match ingress.submit(bytes) {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::channels::telegram::ingress::TelegramIngress;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use std::net::SocketAddr;
    use tower::ServiceExt;

    const SECRET: &str = "s3cret-Token_1";
    const PATH: &str = "/api/webhooks/telegram";

    fn app(ingress: Option<TelegramIngress>) -> Router {
        let pool = crate::db::pool::lazy_pool("postgres://unused@127.0.0.1:1/unused");
        let mut state = crate::interface::http::auth::ApiState::new(
            crate::db::Database { pool },
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        );
        state.telegram = ingress;
        Router::new()
            .route(PATH, post(telegram_update))
            .with_state(state)
    }

    /// Each test uses its own peer address: the limiter is process-wide.
    fn request(ip: &str, secret: Option<&str>, body: Vec<u8>) -> Request<Body> {
        let mut builder = Request::builder().method("POST").uri(PATH);
        if let Some(secret) = secret {
            builder = builder.header("x-telegram-bot-api-secret-token", secret);
        }
        let mut request = builder.body(Body::from(body)).unwrap();
        let addr: SocketAddr = format!("{ip}:4000").parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(addr));
        request
    }

    #[tokio::test]
    async fn without_an_ingress_the_endpoint_is_404() {
        let response = app(None)
            .oneshot(request("198.51.100.1", Some(SECRET), b"{}".to_vec()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_wrong_or_missing_secret_is_401_and_queues_nothing() {
        let (ingress, mut rx) = TelegramIngress::channel(4, Some(SECRET));
        let app = app(Some(ingress));
        for (n, secret) in [Some("wrong"), None, Some("")].into_iter().enumerate() {
            let ip = format!("198.51.100.{}", 10 + n);
            let response = app
                .clone()
                .oneshot(request(&ip, secret, b"{\"update_id\":1}".to_vec()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        assert!(rx.try_recv().is_err(), "nothing may be queued");
    }

    /// No cookie, no `Origin`, no `Authorization`: Telegram is the caller and the
    /// secret is the credential, so the cookie CSRF rule must not apply.
    #[tokio::test]
    async fn the_right_secret_without_origin_or_cookie_queues_the_exact_bytes() {
        let (ingress, mut rx) = TelegramIngress::channel(4, Some(SECRET));
        let body = b"{\"update_id\": 7,  \"message\":null}".to_vec();
        let response = app(Some(ingress))
            .oneshot(request("198.51.100.20", Some(SECRET), body.clone()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(rx.try_recv().unwrap().as_ref(), body.as_slice());
    }

    #[tokio::test]
    async fn a_full_queue_is_503() {
        let (ingress, _rx) = TelegramIngress::channel(1, Some(SECRET));
        let app = app(Some(ingress));
        let first = app
            .clone()
            .oneshot(request("198.51.100.30", Some(SECRET), b"1".to_vec()))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let second = app
            .oneshot(request("198.51.100.30", Some(SECRET), b"2".to_vec()))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn the_121st_request_in_a_minute_is_429_before_the_secret_is_checked() {
        let (ingress, _rx) = TelegramIngress::channel(1, Some(SECRET));
        let app = app(Some(ingress));
        for _ in 0..120 {
            let response = app
                .clone()
                .oneshot(request("198.51.100.40", Some("wrong"), Vec::new()))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        let response = app
            .oneshot(request("198.51.100.40", Some(SECRET), Vec::new()))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn an_oversized_body_is_413_and_queues_nothing() {
        let (ingress, mut rx) = TelegramIngress::channel(4, Some(SECRET));
        let body = vec![b'x'; MAX_UPDATE_BYTES + 1];
        let response = app(Some(ingress))
            .oneshot(request("198.51.100.50", Some(SECRET), body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert!(rx.try_recv().is_err());
    }
}
