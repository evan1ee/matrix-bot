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

pub async fn start_api_server(
    config: &Config,
    db: Database,
    broadcaster: EventBroadcaster,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> (
    std::sync::Arc<tokio::sync::RwLock<Option<String>>>,
    std::sync::Arc<tokio::sync::RwLock<Option<matrix_sdk::Client>>>,
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

    let api_routes = Router::new()
        .route("/health", get(health::health_check))
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .route("/auth/passkey/register/start", post(auth::webauthn::register_start))
        .route("/auth/passkey/register/finish", post(auth::webauthn::register_finish))
        .route("/auth/passkey/login/start", post(auth::webauthn::login_start))
        .route("/auth/passkey/login/finish", post(auth::webauthn::login_finish))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/media", get(auth::media_proxy))
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
        .route("/recurring-bills/:id/pay", post(recurring_bills::pay_bill));

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

    (bot_access_token, bot_client_handle)
}
