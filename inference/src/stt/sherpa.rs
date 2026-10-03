use std::path::{Path, PathBuf};
use tracing::info;

use super::audio::TARGET_SAMPLE_RATE;

pub const SENSEVOICE_MODEL_FILE: &str = "model.int8.onnx";
pub const SENSEVOICE_TOKENS_FILE: &str = "tokens.txt";

/// The `language` values SenseVoice accepts. Anything else makes
/// `OfflineRecognizer::create` fail, so the route rejects it up front.
pub const SENSEVOICE_LANGUAGES: &[&str] = &["auto", "zh", "en", "ja", "ko", "yue"];

use thiserror::Error;

/// One transcription, as the route turns it into a response.
#[derive(Debug, Clone)]
pub struct VoiceTranscriptionResult {
    pub text: String,
    pub language: Option<String>,
    pub duration_secs: f32,
}

#[derive(Debug, Error)]
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
}

impl From<std::io::Error> for VoiceError {
    fn from(err: std::io::Error) -> Self {
        VoiceError::Io(err.to_string())
    }
}

/// Resolves model directory, checking inference subfolder or parent directory.
pub fn resolve_model_dir(dir: &Path) -> PathBuf {
    if dir.exists() {
        dir.to_path_buf()
    } else if let Ok(current) = std::env::current_dir() {
        let candidate_inf = current.join("inference").join(dir);
        if candidate_inf.exists() {
            return candidate_inf;
        }
        if let Some(parent) = current.parent() {
            let candidate = parent.join(dir);
            if candidate.exists() {
                return candidate;
            }
        }
        dir.to_path_buf()
    } else {
        dir.to_path_buf()
    }
}

/// Whether both files SenseVoice needs are on disk. The route gates on this
/// and answers 503 until provisioning has finished.
pub fn models_present(model_dir: &Path) -> bool {
    let resolved = resolve_model_dir(model_dir);
    if resolved.join(SENSEVOICE_MODEL_FILE).is_file()
        && resolved.join(SENSEVOICE_TOKENS_FILE).is_file()
    {
        return true;
    }
    if model_dir == Path::new("models/stt") || model_dir.ends_with("models/stt") {
        let legacy = resolve_model_dir(Path::new("data/voice_models"));
        if legacy.join(SENSEVOICE_MODEL_FILE).is_file()
            && legacy.join(SENSEVOICE_TOKENS_FILE).is_file()
        {
            return true;
        }
    }
    false
}

const MODEL_DOWNLOAD_URL: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx";
const TOKENS_DOWNLOAD_URL: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt";

/// Ensures SenseVoiceSmall ONNX model and tokens are present in `model_dir`,
/// downloading them automatically from Hugging Face if needed.
pub async fn ensure_models_downloaded(model_dir: &Path) -> Result<(), VoiceError> {
    let resolved = resolve_model_dir(model_dir);
    tokio::fs::create_dir_all(&resolved).await.map_err(|e| {
        VoiceError::Io(format!(
            "Failed to create voice model dir {:?}: {}",
            resolved, e
        ))
    })?;

    let files = [
        (SENSEVOICE_MODEL_FILE, MODEL_DOWNLOAD_URL),
        (SENSEVOICE_TOKENS_FILE, TOKENS_DOWNLOAD_URL),
    ];

    for (name, url) in files {
        let dest = resolved.join(name);
        if !dest.exists() {
            info!(
                "SenseVoice model asset {} missing; downloading from {}",
                name, url
            );
            download_model(url, &dest).await?;
            info!("SenseVoice asset {} downloaded to {:?}", name, dest);
        }
    }

    Ok(())
}

async fn download_model(url: &str, dest: &Path) -> Result<(), VoiceError> {
    crate::download::download_to(url, dest)
        .await
        .map_err(|e| VoiceError::ModelDownload(e.0))
}

/// Runs speech recognition synchronously on 16kHz mono f32 samples using sherpa-onnx OfflineRecognizer.
pub fn run_transcription_sync(
    samples: &[f32],
    model_dir: &Path,
    language: &str,
) -> Result<VoiceTranscriptionResult, VoiceError> {
    if samples.is_empty() {
        return Ok(VoiceTranscriptionResult {
            text: String::new(),
            language: None,
            duration_secs: 0.0,
        });
    }

    let resolved_dir = if models_present(model_dir) {
        let r = resolve_model_dir(model_dir);
        if r.join(SENSEVOICE_MODEL_FILE).is_file() && r.join(SENSEVOICE_TOKENS_FILE).is_file() {
            r
        } else {
            resolve_model_dir(Path::new("data/voice_models"))
        }
    } else {
        resolve_model_dir(model_dir)
    };

    let model_path = resolved_dir.join(SENSEVOICE_MODEL_FILE);
    let tokens_path = resolved_dir.join(SENSEVOICE_TOKENS_FILE);

    if !model_path.exists() || !tokens_path.exists() {
        return Err(VoiceError::MissingModel(format!(
            "Required SenseVoice files missing in {:?}. Need {} and {}",
            resolved_dir, SENSEVOICE_MODEL_FILE, SENSEVOICE_TOKENS_FILE
        )));
    }

    let sense_voice = sherpa_onnx::OfflineSenseVoiceModelConfig {
        model: Some(model_path.to_string_lossy().to_string()),
        language: Some(language.to_string()),
        use_itn: true,
    };

    let model_config = sherpa_onnx::OfflineModelConfig {
        sense_voice,
        tokens: Some(tokens_path.to_string_lossy().to_string()),
        num_threads: 2,
        debug: false,
        provider: Some("cpu".to_string()),
        model_type: Some("sense_voice".to_string()),
        ..Default::default()
    };

    let recognizer_config = sherpa_onnx::OfflineRecognizerConfig {
        model_config,
        ..Default::default()
    };

    let recognizer =
        sherpa_onnx::OfflineRecognizer::create(&recognizer_config).ok_or_else(|| {
            VoiceError::Inference("Failed to create sherpa-onnx OfflineRecognizer".to_string())
        })?;

    let stream = recognizer.create_stream();
    stream.accept_waveform(TARGET_SAMPLE_RATE as i32, samples);
    recognizer.decode(&stream);

    let raw_text = stream.get_result().map(|r| r.text).unwrap_or_default();

    let cleaned_text = clean_sensevoice_output(&raw_text);
    let duration_secs = samples.len() as f32 / TARGET_SAMPLE_RATE as f32;

    Ok(VoiceTranscriptionResult {
        text: cleaned_text,
        language: Some(language.to_string()),
        duration_secs,
    })
}

/// Cleans any special tokens or tags sometimes emitted by SenseVoice (e.g. emotion / sound tags like `<|zh|>`, `<|HAPPY|>`).
pub fn clean_sensevoice_output(text: &str) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut in_tag = false;

    for c in text.chars() {
        if c == '<' {
            in_tag = true;
        } else if in_tag {
            if c == '>' {
                in_tag = false;
            }
        } else {
            cleaned.push(c);
        }
    }

    cleaned.trim().to_string()
}

/// Asynchronous wrapper for non-blocking speech recognition on Tokio worker pool.
pub async fn run_transcription(
    samples: Vec<f32>,
    model_dir: PathBuf,
    language: String,
) -> Result<VoiceTranscriptionResult, VoiceError> {
    ensure_models_downloaded(&model_dir).await?;

    tokio::task::spawn_blocking(move || run_transcription_sync(&samples, &model_dir, &language))
        .await
        .map_err(|e| VoiceError::Inference(format!("Worker thread panicked: {}", e)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_sensevoice_output_removes_tags() {
        let raw = "<|zh|><|NEUTRAL|><|Speech|><|woitn|>今天天气真好。";
        let cleaned = clean_sensevoice_output(raw);
        assert_eq!(cleaned, "今天天气真好。");
    }

    #[test]
    fn test_clean_sensevoice_output_plain_text() {
        let raw = "Record an expense for lunch 35 dollars.";
        let cleaned = clean_sensevoice_output(raw);
        assert_eq!(cleaned, "Record an expense for lunch 35 dollars.");
    }

    #[test]
    fn test_models_present_is_false_for_a_missing_directory() {
        assert!(!models_present(Path::new("/nonexistent/voice/models")));
    }

    #[test]
    fn test_models_present_requires_both_the_model_and_the_tokens() {
        let dir = std::env::temp_dir().join(format!("stt-present-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        std::fs::write(dir.join(SENSEVOICE_MODEL_FILE), b"x").unwrap();
        assert!(!models_present(&dir));

        std::fs::write(dir.join(SENSEVOICE_TOKENS_FILE), b"x").unwrap();
        assert!(models_present(&dir));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    #[ignore = "loads the real SenseVoice model; see the doc comment"]
    fn test_real_sensevoice_transcribes_silence_without_error() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/stt");
        let legacy_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/voice_models");
        let model_dir = if dir.exists() && models_present(&dir) {
            dir
        } else {
            legacy_dir
        };
        if !models_present(&model_dir) {
            eprintln!("skipping: no SenseVoice model at {:?}", model_dir);
            return;
        }
        let wav = crate::stt::test_audio::silent_wav_16k_mono(1.0);
        let samples = crate::stt::decode_audio_to_16k_mono(&wav).expect("silent WAV decodes");

        let result = run_transcription_sync(&samples, &model_dir, "auto")
            .expect("SenseVoice should run over silence");

        assert!((result.duration_secs - 1.0).abs() < 0.01);
    }
}
