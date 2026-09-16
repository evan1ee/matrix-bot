use std::fmt;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub model_dir: PathBuf,
    pub language: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model_dir: PathBuf::from("data/voice_models"),
            language: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct VoiceTranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub duration_secs: f32,
}

impl VoiceTranscriptionResult {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum VoiceError {
    #[error("Audio decoding failed: {0}")]
    AudioDecode(String),

    #[error("Missing SenseVoice ONNX model: {0}")]
    MissingModel(String),

    #[error("Failed to download voice model: {0}")]
    ModelDownload(String),

    #[error("Voice transcription inference error: {0}")]
    Inference(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("Voice transcription feature is disabled")]
    Disabled,
}

impl From<std::io::Error> for VoiceError {
    fn from(err: std::io::Error) -> Self {
        VoiceError::Io(err.to_string())
    }
}

impl fmt::Display for VoiceTranscriptionResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.text)
    }
}
