use std::sync::Arc;
use tracing::{info, warn};

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

    // Provision the OCR models before serving, so a fresh deployment self-heals
    // instead of permanently 503ing (`routes::ocr::scan` gates on
    // `models_present` and never downloads). A failure here — no network, a
    // dead mirror — is deliberately non-fatal: the server still comes up and
    // `/health` reports `ocr: false` truthfully, rather than crash-looping on
    // a transient network blip.
    if let Err(err) = paddle::ensure_models(&config.ocr_model_dir).await {
        warn!(
            "OCR model provisioning failed at boot; OCR will report unavailable until this \
             is resolved: {}",
            err
        );
    }

    let state = routes::AppState {
        config: Arc::new(config),
    };

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    info!("Inference server listening on 0.0.0.0:{}", port);

    axum::serve(listener, routes::build_router(state)).await?;
    Ok(())
}
