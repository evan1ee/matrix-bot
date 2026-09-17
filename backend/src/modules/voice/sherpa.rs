use std::path::{Path, PathBuf};
use tracing::info;

use super::audio::TARGET_SAMPLE_RATE;
use super::models::{VoiceError, VoiceTranscriptionResult};

pub const SENSEVOICE_MODEL_FILE: &str = "model.int8.onnx";
pub const SENSEVOICE_TOKENS_FILE: &str = "tokens.txt";

const MODEL_DOWNLOAD_URL: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/model.int8.onnx";
const TOKENS_DOWNLOAD_URL: &str =
    "https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17/resolve/main/tokens.txt";

/// Ensures SenseVoiceSmall ONNX model and tokens are present in `model_dir`,
/// downloading them automatically from Hugging Face if needed.
pub async fn ensure_models_downloaded(model_dir: &Path) -> Result<(), VoiceError> {
    tokio::fs::create_dir_all(model_dir).await.map_err(|e| {
        VoiceError::Io(format!(
            "Failed to create voice model dir {:?}: {}",
            model_dir, e
        ))
    })?;

    let files = [
        (SENSEVOICE_MODEL_FILE, MODEL_DOWNLOAD_URL),
        (SENSEVOICE_TOKENS_FILE, TOKENS_DOWNLOAD_URL),
    ];

    for (name, url) in files {
        let dest = model_dir.join(name);
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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| VoiceError::ModelDownload(e.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| VoiceError::ModelDownload(format!("Failed to request {}: {}", url, e)))?;

    if !response.status().is_success() {
        return Err(VoiceError::ModelDownload(format!(
            "Download failed with HTTP status {}: {}",
            response.status(),
            url
        )));
    }

    let bytes = response.bytes().await.map_err(|e| {
        VoiceError::ModelDownload(format!("Failed to read stream from {}: {}", url, e))
    })?;

    let tmp_dest = dest.with_extension("tmp");
    tokio::fs::write(&tmp_dest, &bytes)
        .await
        .map_err(|e| VoiceError::ModelDownload(format!("Failed to write tmp model file: {}", e)))?;

    tokio::fs::rename(&tmp_dest, dest)
        .await
        .map_err(|e| VoiceError::ModelDownload(format!("Failed to persist model file: {}", e)))?;

    Ok(())
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

    let model_path = model_dir.join(SENSEVOICE_MODEL_FILE);
    let tokens_path = model_dir.join(SENSEVOICE_TOKENS_FILE);

    if !model_path.exists() || !tokens_path.exists() {
        return Err(VoiceError::MissingModel(format!(
            "Required SenseVoice files missing in {:?}. Need {} and {}",
            model_dir, SENSEVOICE_MODEL_FILE, SENSEVOICE_TOKENS_FILE
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
}
