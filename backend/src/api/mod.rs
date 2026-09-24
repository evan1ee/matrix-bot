use axum::{
    http::{header, HeaderValue, Method},
    routing::{get, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;

pub mod account;
pub mod ai;
pub mod auth;
pub mod bookmarks;
pub mod briefing;
pub mod finance;
pub mod finance_analytics;
pub mod finance_net_worth;
pub mod footprints;
pub mod habits;
pub mod health;
pub mod life;
pub mod life_activity;
pub mod mcp;
pub mod ocr;
pub mod openapi;
pub mod portfolio;
pub mod portfolio_alerts;
pub mod portfolio_cash;
pub mod portfolio_history;
pub mod portfolio_prices;
pub mod rate_limit;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod routes;
pub mod settings;
pub mod sse;
pub mod static_files;
pub mod todos;
pub mod voice;
pub mod webhooks;
pub mod webhooks_telegram;

pub use routes::api_routes;

pub async fn start_api_server(
    config: &Config,
    db: Database,
    broadcaster: EventBroadcaster,
    handles: crate::bot::handles::BotHandles,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> anyhow::Result<()> {
    let state = auth::ApiState::from_config(config, db, broadcaster, handles)?;

    // Same-origin by default. `Any`/`Any`/`Any` let any site on the internet
    // script authenticated calls against this API using a token it had obtained;
    // an explicit allowlist keeps that surface to hosts the operator names.
    let mut allowed_origins: Vec<HeaderValue> = config
        .cors_allowed_origins
        .iter()
        .filter_map(|origin| match origin.parse::<HeaderValue>() {
            Ok(value) => Some(value),
            Err(_) => {
                tracing::warn!("Ignoring malformed CORS origin: {}", origin);
                None
            }
        })
        .collect();

    if allowed_origins.is_empty()
        && (config.webauthn_rp_id == "localhost" || config.webauthn_origin.contains("localhost"))
    {
        for origin in &[
            "http://localhost:5173",
            "http://localhost:8080",
            "http://127.0.0.1:5173",
            "http://127.0.0.1:8080",
        ] {
            if let Ok(val) = origin.parse::<HeaderValue>() {
                allowed_origins.push(val);
            }
        }
    }

    if allowed_origins.is_empty() {
        info!("CORS: same-origin only (set CORS_ALLOWED_ORIGINS to permit others)");
    } else {
        info!("CORS: allowing origins {:?}", allowed_origins);
    }

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE])
        .allow_credentials(true);

    let api_routes = api_routes();

    let app = Router::new()
        .nest("/api/v1", api_routes)
        .route(
            "/bot:token/sendMessage",
            get(webhooks::telegram_send_message_get).post(webhooks::telegram_send_message_post),
        )
        .route("/webhooks/:token", post(webhooks::inbound_webhook))
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .fallback(static_files::static_handler)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(180)))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1 MB
        .with_state(state);

    let host = config.api_host.parse().unwrap_or([0, 0, 0, 0].into());
    let addr = SocketAddr::new(host, config.api_port);

    info!("Starting REST API & Web Server at http://{}", addr);

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                let serve_future = axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.recv().await;
                    info!("Axum REST API server received shutdown signal.");
                });

                if let Err(err) = serve_future.await {
                    tracing::error!("REST API server error: {:?}", err);
                }
            }
            Err(err) => {
                tracing::error!("Failed to bind REST API address {}: {:?}", addr, err);
            }
        }
    });

    Ok(())
}

#[cfg(test)]
mod step_up_tests {
    use super::*;
    use crate::accounts::{AccountRepository, SessionRepository, STATUS_ACTIVE};
    use crate::events::EventBroadcaster;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const ORIGIN: &str = "http://localhost:8080";

    /// Every route that changes how the account can be reached.
    ///
    /// This is §6 of the design as an executable list. A new sensitive endpoint
    /// added without `VerifiedUser` fails here rather than shipping, which is
    /// the only thing keeping "sensitive actions need a recent passkey" true
    /// rather than aspirational.
    const SENSITIVE: &[(&str, &str)] = &[
        ("POST", "/api/v1/account/matrix/link"),
        ("POST", "/api/v1/account/matrix/unlink"),
        ("POST", "/api/v1/account/matrix/confirm-merge"),
        ("POST", "/api/v1/account/sessions/revoke-others"),
        ("DELETE", "/api/v1/account/sessions/some-id"),
        ("POST", "/api/v1/account/recovery-codes"),
        ("DELETE", "/api/v1/account/passkeys/some-id"),
        ("PUT", "/api/v1/account/passkeys/some-id"),
    ];

    /// Routes an ordinary, hours-old session must still be able to use.
    const ORDINARY: &[(&str, &str)] = &[
        ("GET", "/api/v1/account/passkeys"),
        ("GET", "/api/v1/account/sessions"),
        ("GET", "/api/v1/account/security-events"),
        ("GET", "/api/v1/account/recovery-status"),
        ("GET", "/api/v1/todos"),
        // Cancelling recovery is deliberately *not* sensitive: the person doing
        // it may be racing a 24-hour clock against a recovery they did not ask
        // for, and demanding a passkey would be exactly backwards.
        ("POST", "/api/v1/account/recovery/cancel"),
    ];

    async fn app_with_session(verified_minutes_ago: i64) -> (Router, String) {
        let pool = crate::db::pool::test_pool().await;
        crate::db::migrate::run_migrations_unlocked(&pool).await;

        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE)
            .await
            .unwrap();
        let token = SessionRepository::create(&pool, &account, None, None, None)
            .await
            .unwrap();

        crate::db_execute!(
            &pool,
            "UPDATE sessions SET last_verified_at = ? WHERE account_id = ?",
            chrono::Utc::now() - chrono::Duration::minutes(verified_minutes_ago),
            &account
        )
        .unwrap();

        let state = auth::ApiState::new(
            crate::db::Database { pool: pool },
            EventBroadcaster::default(),
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        );

        let app = Router::new()
            .nest("/api/v1", api_routes())
            .with_state(state);

        (app, token)
    }

    async fn status_of(app: &Router, method: &str, path: &str, cookie: &str) -> StatusCode {
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(axum::http::header::COOKIE, format!("mb_session={cookie}"))
            .header(axum::http::header::ORIGIN, ORIGIN)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from("{}"))
            .unwrap();

        app.clone().oneshot(request).await.unwrap().status()
    }

    /// A session verified minutes ago must be refused for anything sensitive.
    #[tokio::test]
    async fn test_every_sensitive_route_demands_step_up() {
        // Well outside the five-minute window.
        let (app, cookie) = app_with_session(60).await;

        for (method, path) in SENSITIVE {
            let status = status_of(&app, method, path, &cookie).await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{method} {path} accepted a stale session — it is missing VerifiedUser"
            );
        }
    }

    /// The same stale session must still be able to do ordinary work, or
    /// step-up has quietly become "sign in again every five minutes".
    #[tokio::test]
    async fn test_ordinary_routes_are_unaffected_by_the_step_up_clock() {
        let (app, cookie) = app_with_session(60).await;

        for (method, path) in ORDINARY {
            let status = status_of(&app, method, path, &cookie).await;
            assert_ne!(
                status,
                StatusCode::FORBIDDEN,
                "{method} {path} demanded step-up but should not"
            );
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} rejected a live session"
            );
        }
    }

    /// And a freshly verified session passes the same sensitive routes, so the
    /// test above is measuring the clock rather than a blanket refusal.
    #[tokio::test]
    async fn test_a_fresh_session_passes_step_up() {
        let (app, cookie) = app_with_session(0).await;

        for (method, path) in SENSITIVE {
            let status = status_of(&app, method, path, &cookie).await;
            assert_ne!(
                status,
                StatusCode::FORBIDDEN,
                "{method} {path} refused a session verified moments ago"
            );
        }
    }

    /// No session at all is a 401 everywhere, sensitive or not.
    #[tokio::test]
    async fn test_no_session_is_unauthorized_everywhere() {
        let (app, _) = app_with_session(0).await;

        for (method, path) in SENSITIVE.iter().chain(ORDINARY.iter()) {
            let status = status_of(&app, method, path, "not-a-session").await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{method} {path} answered without a valid session"
            );
        }
    }
}
