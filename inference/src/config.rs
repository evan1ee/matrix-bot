use std::path::PathBuf;

/// Runtime configuration for the inference server, read once at boot.
#[derive(Debug, Clone)]
pub struct InferenceConfig {
    pub port: u16,
    pub ocr_model_dir: PathBuf,
    pub voice_model_dir: PathBuf,
    pub tts_model_dir: PathBuf,
    pub embedding_model_dir: PathBuf,
}

impl Default for InferenceConfig {
    fn default() -> Self {
        Self {
            port: 8090,
            ocr_model_dir: PathBuf::from("models/ocr"),
            voice_model_dir: PathBuf::from("models/stt"),
            tts_model_dir: PathBuf::from("models/tts"),
            embedding_model_dir: PathBuf::from("models/embeddings"),
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

        let voice_model_dir = std::env::var("VOICE_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or(defaults.voice_model_dir);

        let tts_model_dir = std::env::var("TTS_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or(defaults.tts_model_dir);

        let embedding_model_dir = std::env::var("EMBEDDING_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or(defaults.embedding_model_dir);

        Self {
            port,
            ocr_model_dir,
            voice_model_dir,
            tts_model_dir,
            embedding_model_dir,
        }
    }
}
