use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

use crate::audio::TARGET_SAMPLE_RATE;

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

/// Whether both files SenseVoice needs are on disk. The route gates on this
/// and answers 503 until provisioning has finished.
pub fn models_present(model_dir: &Path) -> bool {
    model_dir.join(SENSEVOICE_MODEL_FILE).is_file()
        && model_dir.join(SENSEVOICE_TOKENS_FILE).is_file()
}

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

/// How long to wait for the next chunk of a model download before giving up.
/// This bounds idle time, not total transfer time: a slow-but-steady 239 MB
/// download on a modest link is not a stall, but a connection with no bytes
/// arriving for this long is.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

async fn download_model(url: &str, dest: &Path) -> Result<(), VoiceError> {
    download_model_with_idle(url, dest, DOWNLOAD_IDLE_TIMEOUT).await
}

/// Streams `url` into `dest` via a `.tmp` file that is renamed into place
/// only on success, so a route watching `dest` never sees a partial model.
/// There is deliberately no total-transfer timeout — only `idle`, applied to
/// each chunk — because the model is 239 MB and a total timeout fails
/// deterministically on any link slower than it assumes.
async fn download_model_with_idle(
    url: &str,
    dest: &Path,
    idle: Duration,
) -> Result<(), VoiceError> {
    let tmp_dest = dest.with_extension("tmp");

    match stream_to_file(url, &tmp_dest, idle).await {
        Ok(()) => tokio::fs::rename(&tmp_dest, dest)
            .await
            .map_err(|e| VoiceError::ModelDownload(format!("Failed to persist model file: {}", e))),
        Err(err) => {
            remove_partial_download(&tmp_dest).await;
            Err(err)
        }
    }
}

/// Deletes a partial `.tmp` download after a failed attempt. Already-gone is
/// not an error: the failure may have happened before the file was created.
async fn remove_partial_download(tmp_dest: &Path) {
    if let Err(err) = tokio::fs::remove_file(tmp_dest).await {
        if err.kind() != std::io::ErrorKind::NotFound {
            warn!("Failed to remove partial download {:?}: {}", tmp_dest, err);
        }
    }
}

/// Requests `url` and writes its body to `tmp_dest` chunk by chunk, failing
/// if no chunk arrives within `idle`.
async fn stream_to_file(url: &str, tmp_dest: &Path, idle: Duration) -> Result<(), VoiceError> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| VoiceError::ModelDownload(e.to_string()))?;

    let mut response = client
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

    let mut file = tokio::fs::File::create(tmp_dest).await.map_err(|e| {
        VoiceError::ModelDownload(format!("Failed to create tmp model file: {}", e))
    })?;

    loop {
        let chunk = tokio::time::timeout(idle, response.chunk())
            .await
            .map_err(|_| {
                VoiceError::ModelDownload(format!(
                    "Download stalled: no data received for {:?} from {}",
                    idle, url
                ))
            })?
            .map_err(|e| {
                VoiceError::ModelDownload(format!("Failed to read stream from {}: {}", url, e))
            })?;

        let Some(bytes) = chunk else {
            break;
        };

        file.write_all(&bytes).await.map_err(|e| {
            VoiceError::ModelDownload(format!("Failed to write tmp model file: {}", e))
        })?;
    }

    file.flush()
        .await
        .map_err(|e| VoiceError::ModelDownload(format!("Failed to flush tmp model file: {}", e)))?;

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

    /// Starts a throwaway server on loopback and returns its base URL. The
    /// inference crate has no shared test-server helper (unlike
    /// `backend/src/services/test_support.rs`), so this is inlined here.
    async fn spawn_server(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("ephemeral port should bind");
        let addr = listener
            .local_addr()
            .expect("listener should have an address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{}", addr)
    }

    fn scratch_dest(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sherpa-download-{}-{}.bin",
            tag,
            std::process::id()
        ))
    }

    #[tokio::test]
    async fn test_download_model_with_idle_streams_a_full_body_to_disk() {
        let body: Vec<u8> = vec![7u8; 1024 * 1024];
        let expected = body.clone();
        let app = axum::Router::new().route(
            "/model.bin",
            axum::routing::get(move || {
                let body = body.clone();
                async move { body }
            }),
        );
        let base = spawn_server(app).await;
        let dest = scratch_dest("ok");
        let tmp_dest = dest.with_extension("tmp");
        std::fs::remove_file(&dest).ok();
        std::fs::remove_file(&tmp_dest).ok();

        download_model_with_idle(
            &format!("{}/model.bin", base),
            &dest,
            Duration::from_secs(5),
        )
        .await
        .expect("download of a well-behaved body should succeed");

        let written = std::fs::read(&dest).expect("destination file should exist");
        assert_eq!(written, expected);
        assert!(
            !tmp_dest.exists(),
            "tmp file should not remain after a successful download"
        );

        std::fs::remove_file(&dest).ok();
    }

    #[tokio::test]
    async fn test_download_model_with_idle_fails_and_cleans_up_on_a_stalled_stream() {
        use futures_util::{stream, StreamExt};
        use std::convert::Infallible;

        let app = axum::Router::new().route(
            "/model.bin",
            axum::routing::get(|| async {
                let first = stream::iter(vec![Ok::<Vec<u8>, Infallible>(vec![1, 2, 3])]);
                let stalled = stream::once(async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok::<Vec<u8>, Infallible>(vec![4, 5, 6])
                });
                axum::body::Body::from_stream(first.chain(stalled))
            }),
        );
        let base = spawn_server(app).await;
        let dest = scratch_dest("stall");
        let tmp_dest = dest.with_extension("tmp");
        std::fs::remove_file(&dest).ok();
        std::fs::remove_file(&tmp_dest).ok();

        let result = download_model_with_idle(
            &format!("{}/model.bin", base),
            &dest,
            Duration::from_millis(200),
        )
        .await;

        assert!(
            matches!(result, Err(VoiceError::ModelDownload(_))),
            "a stalled stream must fail as ModelDownload, got {:?}",
            result
        );
        assert!(
            !dest.exists(),
            "destination must not exist after a stalled download"
        );
        assert!(
            !tmp_dest.exists(),
            "tmp file must not remain after a stalled download"
        );
    }

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

    /// Loads the real SenseVoice model, so it is `#[ignore]`d for the reason
    /// `backend/src/modules/tts/engine.rs` gives: on Windows a mismatched
    /// `onnxruntime.dll` kills the whole test process with an access violation.
    /// Run by hand: `cargo test -p matrix-bot-inference real_sensevoice -- --ignored`.
    ///
    /// What it proves is that SenseVoice loads and decodes in the same process
    /// as `ort`'s statically linked ONNX Runtime. The input is silence, so it
    /// cannot assert any words.
    #[test]
    #[ignore = "loads the real SenseVoice model; see the doc comment"]
    fn test_real_sensevoice_transcribes_silence_without_error() {
        let model_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/voice_models");
        if !models_present(&model_dir) {
            eprintln!("skipping: no SenseVoice model at {:?}", model_dir);
            return;
        }
        let wav = crate::audio::test_audio::silent_wav_16k_mono(1.0);
        let samples = crate::audio::decode_audio_to_16k_mono(&wav).expect("silent WAV decodes");

        let result = run_transcription_sync(&samples, &model_dir, "auto")
            .expect("SenseVoice should run over silence");

        assert!((result.duration_secs - 1.0).abs() < 0.01);
    }
}
