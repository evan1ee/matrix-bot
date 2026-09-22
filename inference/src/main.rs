use std::sync::Arc;
use tracing::info;

mod config;
mod paddle;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,matrix_bot_inference=info".into()),
        )
        .init();

    let config = config::InferenceConfig::from_env();
    let port = config.port;

    let state = routes::AppState {
        config: Arc::new(config),
    };

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    info!("Inference server listening on 0.0.0.0:{}", port);

    axum::serve(listener, routes::build_router(state)).await?;
    Ok(())
}
