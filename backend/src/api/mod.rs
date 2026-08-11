use axum::{
    http::{header, HeaderValue, Method},
    routing::{delete, get, patch, post, put},
    Router,
};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::info;

use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;

/// Ceiling on a single outbound homeserver request, start to finish.
const HTTP_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
/// Ceiling on establishing the TCP/TLS connection alone.
const HTTP_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

pub mod account;
pub mod auth;
pub mod health;
pub mod openapi;
pub mod finance;
pub mod portfolio;
pub mod recurring_bills;
pub mod reminders;
pub mod rooms;
pub mod settings;
pub mod sse;
pub mod todos;

/// The API route table.
///
/// Extracted so a test can drive the *real* router rather than a hand-built
/// copy that could drift from it — which is exactly the failure mode a test
/// asserting "every sensitive route demands step-up" has to avoid.
pub fn api_routes() -> Router<auth::ApiState> {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .route("/auth/passkey/register/start", post(auth::webauthn::register_start))
        .route("/auth/passkey/register/finish", post(auth::webauthn::register_finish))
        .route("/auth/passkey/login/start", post(auth::webauthn::login_start))
        .route("/auth/passkey/login/finish", post(auth::webauthn::login_finish))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/media", get(auth::media_proxy))
        .route("/account/passkeys", get(account::list_passkeys))
        .route(
            "/account/passkeys/:id",
            put(account::rename_passkey).delete(account::delete_passkey),
        )
        .route("/account/matrix", get(account::list_links))
        .route("/account/matrix/link", post(account::start_link))
        .route("/account/matrix/pending-merge", get(account::pending_merge))
        .route("/account/matrix/confirm-merge", post(account::confirm_merge))
        .route("/account/matrix/unlink", post(account::unlink))
        .route("/account/sessions", get(account::list_sessions))
        .route("/account/sessions/revoke-others", post(account::revoke_other_sessions))
        .route("/account/recovery-codes", post(account::regenerate_recovery_codes))
        .route("/account/recovery-status", get(account::recovery_status))
        .route("/account/recovery/cancel", post(account::cancel_recovery))
        .route("/account/security-events", get(account::list_security_events))
        .route("/rooms", get(rooms::list_rooms))
        .route("/settings", get(settings::get_settings).put(settings::update_settings))
        .route("/events", get(sse::sse_handler))
        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        .route("/todos/:id", patch(todos::update_todo).delete(todos::delete_todo))
        .route("/reminders", get(reminders::list_reminders).post(reminders::create_reminder))
        .route(
            "/reminders/:id",
            patch(reminders::update_reminder).delete(reminders::delete_reminder),
        )
        .route(
            "/finance/transactions",
            get(finance::list_transactions).post(finance::create_transaction),
        )
        .route(
            "/finance/transactions/:id",
            put(finance::update_transaction).delete(finance::delete_transaction),
        )
        .route("/finance/summary", get(finance::summary))
        .route(
            "/finance/categories",
            get(finance::list_categories).post(finance::create_category),
        )
        .route("/finance/categories/:id", delete(finance::delete_category))
        .route(
            "/portfolio/holdings",
            get(portfolio::list_holdings).post(portfolio::buy_holding),
        )
        .route("/portfolio/holdings/:id", delete(portfolio::delete_holding))
        .route("/portfolio/holdings/:id/sell", post(portfolio::sell_holding))
        .route("/portfolio/holdings/:id/price", put(portfolio::set_price))
        .route("/portfolio/holdings/:id/note", put(portfolio::set_note))
        .route("/portfolio/summary", get(portfolio::summary))
        .route("/portfolio/cash", get(portfolio::list_cash))
        .route("/portfolio/cash/deposit", post(portfolio::deposit_cash))
        .route("/portfolio/cash/withdraw", post(portfolio::withdraw_cash))
        .route(
            "/recurring-bills",
            get(recurring_bills::list_bills).post(recurring_bills::create_bill),
        )
        .route(
            "/recurring-bills/:id",
            get(recurring_bills::get_bill)
                .put(recurring_bills::update_bill)
                .delete(recurring_bills::delete_bill),
        )
        .route("/recurring-bills/:id/pay", post(recurring_bills::pay_bill))
}

pub async fn start_api_server(
    config: &Config,
    db: Database,
    broadcaster: EventBroadcaster,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> (
    std::sync::Arc<tokio::sync::RwLock<Option<String>>>,
    std::sync::Arc<tokio::sync::RwLock<Option<matrix_sdk::Client>>>,
    std::sync::Arc<crate::bot::linking::PendingMerges>,
) {
    let state = auth::ApiState::new(
        db,
        broadcaster,
        config.registration_secret.clone(),
        config.homeserver.clone(),
        // Bound outbound calls so a stalled homeserver cannot pin a request task
        // indefinitely -- but generously. Reaching a homeserver through a VPN or
        // proxy can take several seconds just to establish the TCP connection
        // (measured at ~6s against one such setup), and a connect timeout tighter
        // than that silently breaks profile and media lookups.
        reqwest::Client::builder()
            .timeout(HTTP_TOTAL_TIMEOUT)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
    );

    // The Relying Party is a boot-time decision: an RP ID that does not match
    // the origin the browser loads mints credentials no browser will return, so
    // a bad pair must stop the process rather than surface as failed sign-ins.
    let state = state
        .with_webauthn(
            &config.webauthn_rp_id,
            &config.webauthn_origin,
            config.cookie_secure,
        )
        .expect("WEBAUTHN_RP_ID/WEBAUTHN_ORIGIN must form a valid Relying Party");

    let bot_access_token = state.bot_access_token.clone();
    let pending_merges = state.pending_merges.clone();
    let bot_client_handle = state.bot_client.clone();

    // Same-origin by default. `Any`/`Any`/`Any` let any site on the internet
    // script authenticated calls against this API using a token it had obtained;
    // an explicit allowlist keeps that surface to hosts the operator names.
    let allowed_origins: Vec<HeaderValue> = config
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

    if allowed_origins.is_empty() {
        info!("CORS: same-origin only (set CORS_ALLOWED_ORIGINS to permit others)");
    } else {
        info!("CORS: allowing origins {:?}", config.cors_allowed_origins);
    }

    let cors = CorsLayer::new()
        .allow_origin(allowed_origins)
        .allow_methods([Method::GET, Method::POST, Method::PATCH, Method::DELETE])
        .allow_headers([header::CONTENT_TYPE])
        .allow_credentials(true);

    let api_routes = api_routes();

    let web_dist = if std::path::Path::new("web/dist").exists() {
        "web/dist"
    } else if std::path::Path::new("backend/web/dist").exists() {
        "backend/web/dist"
    } else {
        "web/dist"
    };

    let app = Router::new()
        .nest("/api/v1", api_routes)
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .fallback_service(ServeDir::new(web_dist))
        .layer(cors)
        .with_state(state);

    let host = config.api_host.parse().unwrap_or([0, 0, 0, 0].into());
    let addr = SocketAddr::new(host, config.api_port);

    info!("Starting REST API & Web Server at http://{}", addr);

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                let serve_future = axum::serve(listener, app).with_graceful_shutdown(async move {
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

    (bot_access_token, bot_client_handle, pending_merges)
}

#[cfg(test)]
mod step_up_tests {
    use super::*;
    use crate::accounts::{AccountRepository, SessionRepository, STATUS_ACTIVE};
    use crate::db::schema::run_migrations;
    use crate::events::EventBroadcaster;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use sqlx::SqlitePool;
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

    async fn app_with_session(
        verified_minutes_ago: i64,
    ) -> (Router, String) {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        run_migrations(&pool).await.unwrap();

        let account = AccountRepository::create(&pool, "evan", STATUS_ACTIVE).await.unwrap();
        let token = SessionRepository::create(&pool, &account, None, None, None)
            .await
            .unwrap();

        sqlx::query("UPDATE sessions SET last_verified_at = ? WHERE account_id = ?")
            .bind(chrono::Utc::now() - chrono::Duration::minutes(verified_minutes_ago))
            .bind(&account)
            .execute(&pool)
            .await
            .unwrap();

        let state = auth::ApiState::new(
            crate::db::Database { pool },
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
            assert_ne!(status, StatusCode::UNAUTHORIZED, "{method} {path} rejected a live session");
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
