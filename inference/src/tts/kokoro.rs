//! Kokoro text-to-speech: the model on disk, and the engine.
//!
//! `build_config` was proved against real audio output before this module
//! existed (103 speakers, 24000 Hz, ~1.5x realtime on a debug build), and is
//! kept narrowed to the one Kokoro family `api/tts.rs` needs. Nothing
//! here is a new design.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};
use thiserror::Error;
use tracing::{info, warn};

/// The sherpa-onnx release archive of the multi-language Kokoro model:
/// 103 voices, English and Mandarin, with per-language lexicons and the
/// Chinese text-normalisation FSTs.
const KOKORO_ARCHIVE_URL: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_1.tar.bz2";

/// The one top-level directory inside that archive.
const KOKORO_ARCHIVE_ROOT: &str = "kokoro-multi-lang-v1_1";

/// Where the archive is downloaded and unpacked, beside the model dir.
const ARCHIVE_FILE_NAME: &str = "kokoro-download.tar.bz2";
const STAGING_DIR_NAME: &str = ".kokoro-staging";

/// The files sherpa-onnx cannot build a Kokoro engine without. The lexicons,
/// `espeak-ng-data`, `dict` and FSTs are optional to it, and a missing one
/// degrades pronunciation rather than failing.
pub const KOKORO_REQUIRED_FILES: [&str; 3] = ["model.onnx", "voices.bin", "tokens.txt"];

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("Failed to install the Kokoro TTS model: {0}")]
    ModelDownload(String),
    #[error("Kokoro TTS model is not installed in {0:?}")]
    ModelMissing(PathBuf),
    #[error("TTS engine error: {0}")]
    Engine(String),
    #[error("Ogg Opus encoding failed: {0}")]
    Encode(String),
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

/// Resolves the actual directory holding the Kokoro ONNX and voices files.
pub fn resolve_kokoro_dir(dir: &Path) -> PathBuf {
    let resolved = resolve_model_dir(dir);
    if KOKORO_REQUIRED_FILES
        .iter()
        .all(|name| resolved.join(name).is_file())
    {
        return resolved;
    }
    let subdir = resolved.join(KOKORO_ARCHIVE_ROOT);
    if KOKORO_REQUIRED_FILES
        .iter()
        .all(|name| subdir.join(name).is_file())
    {
        return subdir;
    }
    if dir == Path::new("models/tts") || dir.ends_with("models/tts") {
        let legacy = resolve_model_dir(Path::new("data/tts_models/kokoro-multi-lang-v1_1"));
        if KOKORO_REQUIRED_FILES
            .iter()
            .all(|name| legacy.join(name).is_file())
        {
            return legacy;
        }
    }
    resolved
}

/// Whether every required model file is on disk. The route gates on this and
/// answers 503 until provisioning has finished.
pub fn model_present(dir: &Path) -> bool {
    let kokoro_dir = resolve_kokoro_dir(dir);
    KOKORO_REQUIRED_FILES
        .iter()
        .all(|name| kokoro_dir.join(name).is_file())
}

/// No streaming callback; only the finished audio is wanted.
type NoCallback = fn(&[f32], f32) -> bool;

/// Text-normalisation FSTs for the Kokoro multi-lang model, in the order
/// sherpa-onnx documents. They turn "12.5%" and "2026年9月20日" into something
/// a Mandarin voice can actually say; without them a number is read digit by
/// digit, or in English, and nothing errors — the audio is just wrong.
const RULE_FSTS: &[&str] = &["date-zh.fst", "number-zh.fst", "phone-zh.fst"];

/// The loaded engine, built once and kept for the life of the process.
static ENGINE: OnceLock<Option<OfflineTts>> = OnceLock::new();

/// Serialises every call into the shared `ENGINE`.
static SYNTH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The path, but only if that file is actually present.
fn opt(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    path.exists().then(|| path.to_string_lossy().into_owned())
}

fn build_config(dir: &Path) -> OfflineTtsConfig {
    let lexicons = ["lexicon-us-en.txt", "lexicon-zh.txt"]
        .iter()
        .filter_map(|name| opt(dir, name))
        .collect::<Vec<String>>()
        .join(",");

    let rule_fsts = RULE_FSTS
        .iter()
        .filter_map(|name| opt(dir, name))
        .collect::<Vec<String>>()
        .join(",");

    OfflineTtsConfig {
        model: OfflineTtsModelConfig {
            kokoro: OfflineTtsKokoroModelConfig {
                model: opt(dir, "model.onnx"),
                voices: opt(dir, "voices.bin"),
                tokens: opt(dir, "tokens.txt"),
                data_dir: opt(dir, "espeak-ng-data"),
                lexicon: (!lexicons.is_empty()).then_some(lexicons),
                dict_dir: opt(dir, "dict"),
                ..Default::default()
            },
            num_threads: 2,
            ..Default::default()
        },
        rule_fsts: (!rule_fsts.is_empty()).then_some(rule_fsts),
        ..Default::default()
    }
}

/// Returns the process-wide engine, loading it on the first call.
fn engine(model_dir: &Path) -> Result<&'static OfflineTts, TtsError> {
    if !model_present(model_dir) {
        return Err(TtsError::ModelMissing(model_dir.to_path_buf()));
    }

    let cell = ENGINE.get_or_init(|| {
        let loaded = OfflineTts::create(&build_config(&resolve_kokoro_dir(model_dir)));
        if loaded.is_none() {
            warn!(
                "Kokoro model files are present in {:?} but failed to load; \
                 TTS will report an engine error until the process is restarted \
                 with valid model files",
                model_dir
            );
        }
        loaded
    });

    cell.as_ref().ok_or_else(|| {
        TtsError::Engine(
            "the Kokoro model is present but failed to load; see the server log".to_string(),
        )
    })
}

fn clamp_speaker_id(requested: i64, num_speakers: i32) -> i32 {
    if num_speakers <= 0 {
        return 0;
    }
    let max_valid = num_speakers - 1;
    if requested < 0 {
        0
    } else if requested > i64::from(max_valid) {
        max_valid
    } else {
        requested as i32
    }
}

fn synthesize_sync(model_dir: &Path, speaker_id: i64, text: &str) -> Result<Speech, TtsError> {
    let tts = engine(model_dir)?;

    let num_speakers = tts.num_speakers();
    let sid = clamp_speaker_id(speaker_id, num_speakers);
    if i64::from(sid) != speaker_id {
        warn!(
            "TTS speaker id {} is out of range for this model ({} speakers); using {} instead",
            speaker_id, num_speakers, sid
        );
    }

    let generation = GenerationConfig {
        sid,
        ..Default::default()
    };

    let audio = {
        let _guard = SYNTH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        tts.generate_with_config(text, &generation, None::<NoCallback>)
    }
    .ok_or_else(|| TtsError::Engine("sherpa-onnx TTS generation failed".to_string()))?;

    Ok(Speech {
        samples: audio.samples().to_vec(),
        sample_rate: audio.sample_rate(),
    })
}

/// Synthesised audio: raw samples plus the sample rate they were generated at.
#[derive(Debug, Clone)]
pub struct Speech {
    pub samples: Vec<f32>,
    pub sample_rate: i32,
}

impl Speech {
    pub fn duration(&self) -> std::time::Duration {
        if self.sample_rate <= 0 {
            return std::time::Duration::ZERO;
        }
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}

pub async fn synthesize(
    model_dir: PathBuf,
    text: String,
    speaker_id: i64,
) -> Result<Speech, TtsError> {
    tokio::task::spawn_blocking(move || synthesize_sync(&model_dir, speaker_id, &text))
        .await
        .map_err(|e| TtsError::Engine(format!("Worker thread panicked: {}", e)))?
}

pub async fn ensure_model_downloaded(model_dir: &Path) -> Result<(), TtsError> {
    ensure_model_from(KOKORO_ARCHIVE_URL, model_dir).await
}

async fn ensure_model_from(url: &str, model_dir: &Path) -> Result<(), TtsError> {
    if model_present(model_dir) {
        return Ok(());
    }
    reject_if_occupied(model_dir)?;

    let parent = model_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    tokio::fs::create_dir_all(&parent)
        .await
        .map_err(|e| install_error("create the model parent directory", e))?;

    let archive = parent.join(ARCHIVE_FILE_NAME);
    let staging = parent.join(STAGING_DIR_NAME);

    info!("Kokoro TTS model missing; downloading {}", url);
    let downloaded = crate::download::download_to(url, &archive)
        .await
        .map_err(|e| TtsError::ModelDownload(e.0));

    let installed = match downloaded {
        Ok(()) => {
            let (archive, staging, model_dir) =
                (archive.clone(), staging.clone(), model_dir.to_path_buf());
            tokio::task::spawn_blocking(move || {
                install_from_archive(&archive, &staging, &model_dir)
            })
            .await
            .map_err(|e| TtsError::ModelDownload(format!("Unpack worker panicked: {}", e)))
            .and_then(|result| result)
        }
        Err(err) => Err(err),
    };

    tokio::fs::remove_file(&archive).await.ok();
    tokio::fs::remove_dir_all(&staging).await.ok();

    if installed.is_ok() {
        info!("Kokoro TTS model installed in {:?}", model_dir);
    }
    installed
}

fn install_from_archive(archive: &Path, staging: &Path, model_dir: &Path) -> Result<(), TtsError> {
    if staging.exists() {
        std::fs::remove_dir_all(staging)
            .map_err(|e| install_error("clear the staging directory", e))?;
    }
    std::fs::create_dir_all(staging)
        .map_err(|e| install_error("create the staging directory", e))?;

    let file = std::fs::File::open(archive).map_err(|e| install_error("open the archive", e))?;
    tar::Archive::new(bzip2::read::BzDecoder::new(std::io::BufReader::new(file)))
        .unpack(staging)
        .map_err(|e| install_error("unpack the archive", e))?;

    let root = staging.join(KOKORO_ARCHIVE_ROOT);
    if !model_present(&root) {
        return Err(TtsError::ModelDownload(format!(
            "the archive has no {}/ holding {}",
            KOKORO_ARCHIVE_ROOT,
            KOKORO_REQUIRED_FILES.join(", ")
        )));
    }

    reject_if_occupied(model_dir)?;
    if model_dir.exists() {
        std::fs::remove_dir(model_dir)
            .map_err(|e| install_error("replace the empty model directory", e))?;
    }

    std::fs::rename(&root, model_dir).map_err(|e| install_error("move the model into place", e))
}

fn is_occupied(model_dir: &Path) -> Result<bool, TtsError> {
    if !model_dir.exists() {
        return Ok(false);
    }
    let empty = std::fs::read_dir(model_dir)
        .map_err(|e| install_error("inspect the model directory", e))?
        .next()
        .is_none();
    Ok(!empty)
}

fn reject_if_occupied(model_dir: &Path) -> Result<(), TtsError> {
    if is_occupied(model_dir)? {
        return Err(TtsError::ModelDownload(format!(
            "{:?} exists but does not hold the model; remove it or point TTS_MODEL_DIR elsewhere",
            model_dir
        )));
    }
    Ok(())
}

fn install_error(action: &str, err: std::io::Error) -> TtsError {
    TtsError::ModelDownload(format!("Failed to {}: {}", action, err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kokoro-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn archive_with(files: &[&str]) -> Vec<u8> {
        let mut tar_bytes = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_bytes);
            for name in files {
                let data = b"x";
                let mut header = tar::Header::new_gnu();
                header.set_size(data.len() as u64);
                header.set_mode(0o644);
                builder
                    .append_data(
                        &mut header,
                        format!("{}/{}", KOKORO_ARCHIVE_ROOT, name),
                        &data[..],
                    )
                    .unwrap();
            }
            builder.finish().unwrap();
        }
        let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        encoder.write_all(&tar_bytes).unwrap();
        encoder.finish().unwrap()
    }

    async fn serve_archive(bytes: Vec<u8>) -> String {
        let app = axum::Router::new().route(
            "/kokoro.tar.bz2",
            axum::routing::get(move || {
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
        format!(
            "{}/kokoro.tar.bz2",
            crate::test_support::spawn_server(app).await
        )
    }

    #[test]
    fn test_model_present_needs_every_required_file() {
        let dir = scratch("present");
        assert!(!model_present(&dir));
        for name in KOKORO_REQUIRED_FILES {
            assert!(!model_present(&dir), "incomplete before {}", name);
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert!(model_present(&dir));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn test_an_installed_model_is_never_downloaded_again() {
        let root = scratch("noop");
        let model_dir = root.join(KOKORO_ARCHIVE_ROOT);
        std::fs::create_dir_all(&model_dir).unwrap();
        for name in KOKORO_REQUIRED_FILES {
            std::fs::write(model_dir.join(name), b"x").unwrap();
        }
        ensure_model_from("http://127.0.0.1:1/never", &model_dir)
            .await
            .expect("a present model must short-circuit");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_installs_the_archive_root_as_the_model_dir_and_cleans_up() {
        let root = scratch("install");
        let model_dir = root.join("kokoro");
        let url = serve_archive(archive_with(&[
            "model.onnx",
            "voices.bin",
            "tokens.txt",
            "espeak-ng-data/phontab",
        ]))
        .await;

        ensure_model_from(&url, &model_dir)
            .await
            .expect("a well-formed archive should install");

        assert!(model_present(&model_dir));
        assert!(model_dir.join("espeak-ng-data/phontab").is_file());
        assert!(
            !root.join(ARCHIVE_FILE_NAME).exists(),
            "archive must be removed"
        );
        assert!(
            !root.join(STAGING_DIR_NAME).exists(),
            "staging must be removed"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_an_archive_missing_required_files_installs_nothing() {
        let root = scratch("incomplete");
        let model_dir = root.join("kokoro");
        let url = serve_archive(archive_with(&["model.onnx"])).await;

        let result = ensure_model_from(&url, &model_dir).await;

        assert!(
            matches!(result, Err(TtsError::ModelDownload(_))),
            "{:?}",
            result
        );
        assert!(!model_dir.exists());
        assert!(!root.join(STAGING_DIR_NAME).exists());
        assert!(!root.join(ARCHIVE_FILE_NAME).exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_an_incomplete_non_empty_model_dir_is_left_alone() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let root = scratch("occupied");
        let model_dir = root.join("kokoro");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("notes.txt"), b"mine").unwrap();

        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = Arc::clone(&hits);
        let bytes = archive_with(&KOKORO_REQUIRED_FILES);
        let app = axum::Router::new().route(
            "/kokoro.tar.bz2",
            axum::routing::get(move || {
                hits_clone.fetch_add(1, Ordering::SeqCst);
                let bytes = bytes.clone();
                async move { bytes }
            }),
        );
        let url = format!(
            "{}/kokoro.tar.bz2",
            crate::test_support::spawn_server(app).await
        );

        let result = ensure_model_from(&url, &model_dir).await;

        assert!(
            matches!(result, Err(TtsError::ModelDownload(_))),
            "{:?}",
            result
        );
        assert_eq!(
            hits.load(Ordering::SeqCst),
            0,
            "the archive server must never be contacted for an occupied directory"
        );
        assert_eq!(std::fs::read(model_dir.join("notes.txt")).unwrap(), b"mine");
        assert!(!root.join(ARCHIVE_FILE_NAME).exists());
        assert!(!root.join(STAGING_DIR_NAME).exists());
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_present_but_corrupt_model_files_report_an_engine_error_not_missing() {
        let dir = scratch("corrupt");
        for name in KOKORO_REQUIRED_FILES {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        assert!(model_present(&dir));

        let result = synthesize(dir.clone(), "hi".to_string(), 0).await;

        assert!(matches!(result, Err(TtsError::Engine(_))), "{:?}", result);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_opt_returns_none_for_a_missing_file() {
        let dir = std::env::temp_dir();
        assert_eq!(opt(&dir, "definitely-not-a-real-file.onnx"), None);
    }

    #[test]
    fn test_clamp_speaker_id_leaves_in_range_ids_untouched() {
        assert_eq!(clamp_speaker_id(0, 103), 0);
        assert_eq!(clamp_speaker_id(50, 103), 50);
        assert_eq!(clamp_speaker_id(102, 103), 102);
    }

    #[test]
    fn test_clamp_speaker_id_clamps_negative_to_zero() {
        assert_eq!(clamp_speaker_id(-1, 103), 0);
        assert_eq!(clamp_speaker_id(-999, 103), 0);
    }

    #[test]
    fn test_clamp_speaker_id_clamps_too_large_to_the_last_valid_index() {
        assert_eq!(clamp_speaker_id(103, 103), 102);
        assert_eq!(clamp_speaker_id(9999, 103), 102);
    }

    #[test]
    fn test_clamp_speaker_id_handles_a_model_with_no_reported_speakers() {
        assert_eq!(clamp_speaker_id(5, 0), 0);
        assert_eq!(clamp_speaker_id(0, 0), 0);
        assert_eq!(clamp_speaker_id(5, -1), 0);
    }

    #[test]
    fn test_duration_matches_sample_count_and_rate() {
        let speech = Speech {
            samples: vec![0.0; 24000],
            sample_rate: 24000,
        };
        assert_eq!(speech.duration(), std::time::Duration::from_secs(1));
    }

    #[test]
    fn test_duration_is_zero_for_an_invalid_sample_rate() {
        let speech = Speech {
            samples: vec![0.0; 100],
            sample_rate: 0,
        };
        assert_eq!(speech.duration(), std::time::Duration::ZERO);
    }

    #[tokio::test]
    #[ignore = "needs a real Kokoro model under models/tts/, and on Windows the sherpa onnxruntime.dll copied into target/debug/deps/"]
    async fn test_synthesize_produces_audio_for_english_and_chinese() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models/tts");
        let legacy_dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/tts_models/kokoro-multi-lang-v1_1");
        let model_dir = if dir.exists() && model_present(&dir) {
            dir
        } else {
            legacy_dir
        };
        if !model_present(&model_dir) {
            eprintln!("Skipping: no Kokoro model present at {:?}", model_dir);
            return;
        }

        let english = synthesize(
            model_dir.clone(),
            "Good morning, this is a test.".to_string(),
            0,
        )
        .await
        .expect("english synthesis should succeed with a real model present");
        assert!(!english.samples.is_empty());
        assert_eq!(english.sample_rate, 24000);

        let chinese = synthesize(model_dir.clone(), "早安，这是一个测试。".to_string(), 0)
            .await
            .expect("chinese synthesis should succeed with a real model present");
        assert!(!chinese.samples.is_empty());
        assert_eq!(chinese.sample_rate, 24000);
    }
}
