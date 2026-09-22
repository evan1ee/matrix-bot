use std::path::PathBuf;

/// Runtime configuration for the inference server, read once at boot.
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    pub port: u16,
    pub ocr_model_dir: PathBuf,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            port: 8090,
            ocr_model_dir: PathBuf::from("data/ocr_models"),
        }
    }
}

impl InferenceConfig {
    pub fn from_env() -> Self {
        let defaults = Self::default();

        let port = std::env::var("INFERENCE_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(defaults.port);

        let ocr_model_dir = std::env::var("OCR_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or(defaults.ocr_model_dir);

        Self {
            port,
            ocr_model_dir,
        }
    }
}
