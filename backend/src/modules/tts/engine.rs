//! The sherpa-onnx Kokoro wrapper.
//!
//! Config construction mirrors `backend/examples/tts_spike.rs`, which proved
//! this exact Kokoro model directory against real audio output (103 speakers,
//! 24000 Hz, ~1.5x realtime on a debug build). Nothing here is a new design —
//! it is that spike's `build_config` narrowed to the one family this module
//! supports.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsKokoroModelConfig,
    OfflineTtsModelConfig,
};

use super::models::{Speech, TtsConfig, TtsError};

/// No streaming callback; only the finished audio is wanted.
type NoCallback = fn(&[f32], f32) -> bool;

/// Text-normalisation FSTs for the Kokoro multi-lang model, in the order
/// sherpa-onnx documents. They turn "12.5%" and "2026年9月20日" into something
/// a Mandarin voice can actually say; without them a number is read digit by
/// digit, or in English, and nothing errors — the audio is just wrong.
const RULE_FSTS: &[&str] = &["date-zh.fst", "number-zh.fst", "phone-zh.fst"];

/// The loaded engine, built once and kept for the life of the process.
///
/// `modules/voice/sherpa.rs::run_transcription_sync` rebuilds its recogniser
/// on every call, which is fine there because that model loads in
/// milliseconds. This Kokoro model is 311 MB and loading it takes seconds, so
/// rebuilding it per `!say` would make every reply pay that cost. `OfflineTts`
/// is `unsafe impl Send + Sync` in the sherpa-onnx crate (it wraps a C++
/// engine that owns no thread-local state), so one instance shared across the
/// async runtime's worker threads is sound. Do not "fix" this back to
/// rebuilding on every call — that was tried in `modules/voice` for a much
/// smaller model and is not the right trade-off here.
static ENGINE: OnceLock<Option<OfflineTts>> = OnceLock::new();

/// The path, but only if that file is actually present.
///
/// Every one of these fields is optional in sherpa-onnx, and handing it a
/// path to something missing fails less helpfully than leaving it unset.
fn opt(dir: &Path, name: &str) -> Option<String> {
    let path = dir.join(name);
    path.exists().then(|| path.to_string_lossy().into_owned())
}

/// Whether the Kokoro model's required files are present in `dir`, checked
/// without loading anything. Used both to decide whether `!say` should even
/// attempt synthesis and, once, to decide whether `ENGINE` should try to load.
pub fn model_present(dir: &Path) -> bool {
    dir.join("model.onnx").exists()
        && dir.join("voices.bin").exists()
        && dir.join("tokens.txt").exists()
}

fn build_config(dir: &Path) -> OfflineTtsConfig {
    // One lexicon per language, comma-joined — sherpa-onnx picks per
    // utterance. That is the whole of "per-language routing" for this engine:
    // a path, not an abstraction.
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
                // espeak-ng data: the grapheme-to-phoneme step, already solved.
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
///
/// A missing model directory is cached as `None` rather than retried on every
/// `!say` — that mirrors `model_present`'s own check, which callers are
/// expected to run first so this path is only reached when the files were
/// seen to exist.
fn engine(model_dir: &Path) -> Result<&'static OfflineTts, TtsError> {
    let cell = ENGINE.get_or_init(|| {
        if !model_present(model_dir) {
            return None;
        }
        OfflineTts::create(&build_config(model_dir))
    });

    cell.as_ref()
        .ok_or_else(|| TtsError::ModelMissing(model_dir.to_path_buf()))
}

fn synthesize_sync(model_dir: &Path, speaker_id: i32, text: &str) -> Result<Speech, TtsError> {
    let tts = engine(model_dir)?;

    let generation = GenerationConfig {
        sid: speaker_id,
        ..Default::default()
    };

    let audio = tts
        .generate_with_config(text, &generation, None::<NoCallback>)
        .ok_or_else(|| TtsError::Engine("sherpa-onnx TTS generation failed".to_string()))?;

    Ok(Speech {
        samples: audio.samples().to_vec(),
        sample_rate: audio.sample_rate(),
    })
}

/// Synthesises `text` into speech using the configured Kokoro model and
/// speaker.
///
/// Runs on a blocking worker thread. Inference is synchronous and CPU-bound
/// for seconds at a time; on the runtime thread it would stall the Matrix
/// sync loop, exactly as `modules/voice/sherpa.rs::run_transcription` already
/// avoids for speech-to-text.
pub async fn synthesize(config: &TtsConfig, text: &str) -> Result<Speech, TtsError> {
    if !config.enabled {
        return Err(TtsError::Disabled);
    }

    let model_dir: PathBuf = config.model_dir.clone();
    let speaker_id = config.speaker_id;
    let text = text.to_string();

    tokio::task::spawn_blocking(move || synthesize_sync(&model_dir, speaker_id, &text))
        .await
        .map_err(|e| TtsError::Engine(format!("Worker thread panicked: {}", e)))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_present_is_false_for_a_missing_directory() {
        assert!(!model_present(Path::new(
            "data/tts_models/definitely-not-here"
        )));
    }

    #[test]
    fn test_opt_returns_none_for_a_missing_file() {
        let dir = std::env::temp_dir();
        assert_eq!(opt(&dir, "definitely-not-a-real-file.onnx"), None);
    }

    /// Requires a real Kokoro model on disk.
    ///
    /// On a Windows development machine it also needs the right
    /// `onnxruntime.dll` beside the test binary. sherpa-onnx's build script
    /// copies it to `target/debug/` and `target/debug/examples/`, but not to
    /// `target/debug/deps/`, where test binaries actually run. Windows then
    /// falls back to whatever `C:\Windows\System32\onnxruntime.dll` some other
    /// installer left behind, and a version mismatch there does not fail
    /// cleanly: the process dies with an access violation partway through model
    /// loading. If that happens:
    ///
    /// ```text
    /// cp target/debug/onnxruntime.dll target/debug/deps/
    /// ```
    ///
    /// This is why `modules/voice` has no test that loads its model either.
    ///
    /// Deployment is unaffected: the Dockerfile copies
    /// `libsherpa-onnx-c-api.so` and `libonnxruntime.so` into `/usr/local/lib`
    /// explicitly, so there is no system copy to be shadowed by.
    #[tokio::test]
    #[ignore = "needs a real Kokoro model under data/tts_models/, and on Windows the sherpa onnxruntime.dll copied into target/debug/deps/"]
    async fn test_synthesize_produces_audio_for_english_and_chinese() {
        let config = TtsConfig {
            enabled: true,
            model_dir: PathBuf::from("data/tts_models/kokoro-multi-lang-v1_1"),
            speaker_id: 0,
        };

        let english = synthesize(&config, "Good morning, this is a test.")
            .await
            .expect("english synthesis should succeed with a real model present");
        assert!(!english.samples.is_empty());
        assert_eq!(english.sample_rate, 24000);

        let chinese = synthesize(&config, "早安，这是一个测试。")
            .await
            .expect("chinese synthesis should succeed with a real model present");
        assert!(!chinese.samples.is_empty());
        assert_eq!(chinese.sample_rate, 24000);
    }
}
