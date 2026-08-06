use axum::{
    routing::{delete, get, patch, post},
    Router,
};
use std::net::SocketAddr;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::info;

use crate::config::Config;
use crate::db::Database;
use crate::events::EventBroadcaster;

pub mod auth;
pub mod health;
pub mod openapi;
pub mod reminders;
pub mod sse;
pub mod todos;

pub async fn start_api_server(config: &Config, db: Database, broadcaster: EventBroadcaster) -> std::sync::Arc<tokio::sync::RwLock<Option<String>>> {
    let state = auth::ApiState::new(
        db,
        broadcaster,
        config.jwt_secret.clone(),
        config.registration_secret.clone(),
        config.homeserver.clone(),
        reqwest::Client::new(),
    );

    let matrix_access_token = state.matrix_access_token.clone();

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/health", get(health::health_check))
        .route("/api-docs/openapi.json", get(openapi::openapi_json))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
        .route("/auth/me", get(auth::get_current_user))
        .route("/auth/media", get(auth::media_proxy))
        .route("/events", get(sse::sse_handler))
        .route("/todos", get(todos::list_todos).post(todos::create_todo))
        .route("/todos/:id", patch(todos::update_todo).delete(todos::delete_todo))
        .route("/reminders", get(reminders::list_reminders).post(reminders::create_reminder))
        .route("/reminders/:id", delete(reminders::delete_reminder));

    let app = Router::new()
        .nest("/api/v1", api_routes)
        .fallback_service(ServeDir::new("web/dist").fallback(ServeDir::new("web")))
        .layer(cors)
        .with_state(state);

    let host = config.api_host.parse().unwrap_or([0, 0, 0, 0].into());
    let addr = SocketAddr::new(host, config.api_port);

    info!("Starting REST API & Web Server at http://{}", addr);

    tokio::spawn(async move {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                if let Err(err) = axum::serve(listener, app).await {
                    tracing::error!("REST API server error: {:?}", err);
                }
            }
            Err(err) => {
                tracing::error!("Failed to bind REST API address {}: {:?}", addr, err);
            }
        }
    });

    matrix_access_token
}
