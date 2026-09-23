use std::sync::Arc;
use tracing::{info, warn};

use crate::config::InferenceConfig;

/// Downloads any model a capability is missing.
///
/// Spawned after the listener binds rather than awaited before it, so a fresh
/// deployment answers `/health` at once instead of reading unhealthy for the
/// minutes a download takes. Each route gates on its own `models_present` and
/// answers 503 until its files land; downloads write to a temporary file and
/// rename, so a route never sees a partial model. A failure is logged and
/// otherwise ignored: `/health` then reports that capability as absent.
pub async fn provision_models(config: Arc<InferenceConfig>) {
    match crate::paddle::ensure_models(&config.ocr_model_dir).await {
        Ok(()) => info!("OCR models ready in {:?}", config.ocr_model_dir),
        Err(err) => warn!(
            "OCR model provisioning failed; OCR reports unavailable until this is resolved: {}",
            err
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A directory already holding every model file, so provisioning has
    /// nothing to download. Tests must never reach the network.
    fn provisioned_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("provision-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir should be creatable");
        for name in [
            crate::paddle::DET_MODEL_NAME,
            crate::paddle::CLS_MODEL_NAME,
            crate::paddle::REC_MODEL_NAME,
        ] {
            std::fs::write(dir.join(name), b"x").expect("model stub should be writable");
        }
        dir
    }

    #[tokio::test]
    async fn test_provisioning_is_a_no_op_when_every_model_is_already_present() {
        let dir = provisioned_dir("noop");
        let config = Arc::new(InferenceConfig {
            port: 0,
            ocr_model_dir: dir.clone(),
        });

        tokio::time::timeout(Duration::from_secs(5), provision_models(config))
            .await
            .expect("provisioning with every file present must finish without downloading");

        std::fs::remove_dir_all(&dir).ok();
    }
}
