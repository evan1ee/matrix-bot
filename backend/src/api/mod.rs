use axum::{
    http::{header, HeaderValue, Method},
    routing::{delete, get, patch, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;
use tracing::info;

use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;

pub mod auth;
pub mod health;
pub mod openapi;
pub mod reminders;
pub mod rooms;
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
        config.jwt_secret.clone(),
        config.registration_secret.clone(),
        config.homeserver.clone(),
        // Always bound outbound calls: a slow homeserver must not be able to pin
        // a request task indefinitely.
        reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
    );

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
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE]);

    let api_routes = Router::new()
        .route("/health", get(health::health_check))
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/media", get(auth::media_proxy))
        .route("/rooms", get(rooms::list_rooms))
        .route("/events", get(sse::sse_handler))
        .route("/events/ticket", post(auth::create_sse_ticket))
        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        .route("/todos/:id", patch(todos::update_todo).delete(todos::delete_todo))
        .route("/reminders", get(reminders::list_reminders).post(reminders::create_reminder))
        .route("/reminders/:id", delete(reminders::delete_reminder));

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
