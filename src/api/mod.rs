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
pub mod reminders;
pub mod sse;
pub mod todos;

pub async fn start_api_server(config: &Config, db: Database, broadcaster: EventBroadcaster) {
    let state = auth::ApiState {
        db,
        broadcaster,
        jwt_secret: config.jwt_secret.clone(),
        registration_secret: config.registration_secret.clone(),
    };

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let api_routes = Router::new()
        .route("/health", get(health::health_check))
        .route("/auth/register", post(auth::register))
        .route("/auth/login", post(auth::login))
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
}
