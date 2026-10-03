use anyhow::Context;
use axum::{
    http::{header, HeaderValue, Method},
    routing::{get, post},
    Router,
};
use std::net::{IpAddr, SocketAddr};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::info;

use crate::config::Config;
use crate::db::Database;

pub mod account;
pub mod agent;
pub mod ai;
pub mod app_error;
pub mod auth;
pub mod bookmarks;
pub mod briefing;
pub mod error;
pub mod finance;
pub mod finance_analytics;
pub mod finance_categories;
pub mod finance_net_worth;
pub mod footprints;
pub mod habits;
pub mod health;
pub mod identities;
#[cfg(test)]
mod identity_api_tests;
pub mod life;
pub mod life_activity;
pub mod link;
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
pub mod storage;
pub mod telegram_webhook;
pub mod todos;
pub mod voice;
pub mod webhooks;

pub use error::{ApiError, ApiErrorResponse};
pub use routes::api_routes;

/// Parses `API_HOST` into an [`IpAddr`].
///
/// A malformed value used to fall back silently to `0.0.0.0` (`unwrap_or`),
/// which meant a typo'd `API_HOST` bound the API to every interface instead
/// of failing boot — the opposite of what an operator who set it would want.
/// Consistent with the rest of boot: a bad config value is a boot error, not
/// a silent fallback.
fn parse_api_host(host: &str) -> anyhow::Result<IpAddr> {
    host.parse()
        .with_context(|| format!("invalid API_HOST {host:?}: not an IP address"))
}

/// Binds the configured `API_HOST:API_PORT`.
///
/// Separate from [`start_api_server`] so a bind failure stops boot instead of
/// leaving a bot running with no API (and no health probes), and so tests can
/// hand the server a listener on an ephemeral port.
pub async fn bind(config: &Config) -> anyhow::Result<TcpListener> {
    let host = parse_api_host(&config.api_host)?;
    let addr = SocketAddr::new(host, config.api_port);
    let listener = TcpListener::bind(&addr)
        .await
        .with_context(|| format!("failed to bind the REST API address {addr}"))?;
    info!("Starting REST API & Web Server at http://{}", addr);
    Ok(listener)
}

/// Serves the API on `listener` until `shutdown` is cancelled.
///
/// Returns the server task so the composition root can bound its shutdown;
/// it completes once graceful shutdown has drained open connections. SSE
/// streams end on the same token (see `sse.rs`), so an open browser tab does
/// not hold the drain open.
#[allow(clippy::too_many_arguments)]
pub fn start_api_server(
    config: &Config,
    db: Database,
    services: crate::application::AppServices,
    hub: crate::events::Hub,
    handles: crate::interface::channels::matrix::handles::BotHandles,
    telegram: Option<crate::interface::channels::telegram::ingress::TelegramIngress>,
    listener: TcpListener,
    shutdown: CancellationToken,
) -> anyhow::Result<JoinHandle<()>> {
    let mut state =
        auth::ApiState::from_config(config, db, services, hub, handles, shutdown.clone())?;
    state.telegram = telegram;

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
        // Not in OpenAPI (T12); 404 unless Telegram runs in webhook mode.
        .route(
            "/api/webhooks/telegram",
            post(telegram_webhook::telegram_update),
        )
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        // Infrastructure surface, not application API: no auth, not under
        // `/api/v1`, so a load balancer or orchestrator probe needs no session.
        .route("/health/live", get(health::live))
        .route("/health/ready", get(health::ready))
        .fallback(static_files::static_handler)
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(TimeoutLayer::new(std::time::Duration::from_secs(180)))
        .layer(RequestBodyLimitLayer::new(1024 * 1024)) // 1 MB
        .with_state(state);

    Ok(tokio::spawn(async move {
        let serve = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown.cancelled().await;
            info!("Axum REST API server received shutdown signal.");
        });

        if let Err(err) = serve.await {
            tracing::error!("REST API server error: {:?}", err);
        }
    }))
}

#[cfg(test)]
mod step_up_tests {
    use super::*;
    use crate::domain::accounts::STATUS_ACTIVE;
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
        ("POST", "/api/v1/account/identities/unlink"),
        ("POST", "/api/v1/link/claim"),
        ("POST", "/api/v1/account/sessions/revoke-others"),
        ("DELETE", "/api/v1/account/sessions/some-id"),
        ("POST", "/api/v1/account/recovery-codes"),
        ("DELETE", "/api/v1/account/passkeys/some-id"),
        ("PUT", "/api/v1/account/passkeys/some-id"),
    ];

    /// Routes an ordinary, hours-old session must still be able to use.
    const ORDINARY: &[(&str, &str)] = &[
        ("GET", "/api/v1/account/passkeys"),
        ("GET", "/api/v1/account/identities"),
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

        let state = auth::ApiState::new(
            crate::db::Database { pool },
            None,
            "https://mbm.one".to_string(),
            reqwest::Client::new(),
        );

        let accounts = &state.services.accounts;
        let account = accounts.create("evan", STATUS_ACTIVE).await.unwrap();
        let token = accounts
            .create_session(&account, None, None, None)
            .await
            .unwrap();

        accounts
            .update_last_verified_at(
                &account,
                chrono::Utc::now() - chrono::Duration::minutes(verified_minutes_ago),
            )
            .await
            .unwrap();

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

#[cfg(test)]
mod api_host_tests {
    use super::parse_api_host;
    use std::net::IpAddr;

    #[test]
    fn accepts_an_ipv4_address() {
        assert_eq!(
            parse_api_host("127.0.0.1").unwrap(),
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn accepts_an_ipv6_address() {
        assert_eq!(
            parse_api_host("::1").unwrap(),
            "::1".parse::<IpAddr>().unwrap()
        );
    }

    #[test]
    fn rejects_a_malformed_host_instead_of_defaulting_to_all_interfaces() {
        // The old `unwrap_or([0, 0, 0, 0].into())` silently bound every
        // interface on a typo; this must be a boot error instead.
        assert!(parse_api_host("not-an-ip").is_err());
        assert!(parse_api_host("").is_err());
        assert!(parse_api_host("localhost").is_err());
    }
}

#[cfg(test)]
mod server_shutdown_tests {
    use super::*;
    use std::time::Duration;

    fn local_config() -> Config {
        Config {
            homeserver: "https://matrix.example.com".to_string(),
            username: "bot".to_string(),
            password: "secretpassword".to_string(),
            recovery_key: None,
            verification_allowlist: Vec::new(),
            database_url: "postgres://unused@127.0.0.1:1/unused".to_string(),
            api_host: "127.0.0.1".to_string(),
            api_port: 0,
            registration_secret: None,
            registration_open: false,
            db_max_connections: 1,
            job_workers: 4,
            job_worker_host: "test".into(),
            cors_allowed_origins: Vec::new(),
            webauthn_rp_id: "localhost".to_string(),
            webauthn_origin: "http://localhost:8080".to_string(),
            cookie_secure: false,
            web_url: "http://localhost:8080".to_string(),
            link_signup_allowlist: Default::default(),
            discord: None,
            telegram: None,
            email: None,
            ocr_enabled: false,
            ocr_base_url: None,
            voice_enabled: false,
            voice_language: "auto".to_string(),
            stt_base_url: None,
            stt_api_key: None,
            stt_model: "sensevoice-small".to_string(),
            tts_base_url: None,
            tts_api_key: None,
            tts_model: "kokoro".to_string(),
            tts_voice: "0".to_string(),
            tts_dialect: crate::infrastructure::inference::tts::TtsDialect::OpenAi,
            trust_forwarded_for: false,
            ai: crate::config::AiEnvConfig::default(),
            storage: crate::config::StorageConfig::default(),
        }
    }

    /// The server task `start_api_server` returns is what the composition root
    /// bounds its shutdown on, so it must actually finish once the token is
    /// cancelled — including with a kept-alive client connection open.
    #[tokio::test]
    async fn test_server_task_completes_promptly_after_cancel() {
        let config = local_config();
        // Lazy: nothing here touches the database, so the test needs no server.
        let pool = crate::db::pool::lazy_pool(&config.database_url);
        let listener = bind(&config).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let shutdown = CancellationToken::new();

        let server = start_api_server(
            &config,
            Database { pool: pool.clone() },
            crate::application::AppServices::new(&pool),
            crate::events::Hub::new(crate::application::AppServices::new(&pool).outbox, 8),
            crate::interface::channels::matrix::handles::BotHandles::default(),
            None,
            listener,
            shutdown.clone(),
        )
        .unwrap();

        // Serving, and leaves a keep-alive connection in the client's pool.
        let client = reqwest::Client::new();
        let live = client
            .get(format!("http://{addr}/health/live"))
            .send()
            .await
            .unwrap();
        assert_eq!(live.status(), reqwest::StatusCode::OK);

        shutdown.cancel();

        let stopped = tokio::time::timeout(Duration::from_secs(1), server).await;
        assert!(
            matches!(stopped, Ok(Ok(()))),
            "the server task must finish within 1 s of cancel: {stopped:?}"
        );
        drop(client);
    }
}
