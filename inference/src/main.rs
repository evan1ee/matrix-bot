use std::sync::Arc;
use tracing::info;

mod api;
mod config;
mod download;
mod embeddings;
mod ocr;
mod provision;
mod stt;
#[cfg(test)]
mod test_support;
mod tts;

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

    let state = api::AppState {
        config: Arc::new(config),
    };

    // Provisioning downloads models and can take minutes; run it beside the
    // server, not in front of it. See `provision::provision_models`.
    tokio::spawn(provision::provision_models(state.config.clone()));

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    info!("Inference server listening on 0.0.0.0:{}", port);

    axum::serve(listener, api::build_router(state)).await?;
    Ok(())
}
