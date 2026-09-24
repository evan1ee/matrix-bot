# Inference TTS (Plan 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move Kokoro text-to-speech out of the bot into `matrix-bot-inference` behind OpenAI's `POST /v1/audio/speech`, give the bot a `services/tts` client that speaks both the OpenAI and ElevenLabs dialects, migrate the per-user voice from `tts_speaker_id: Option<i64>` to `tts_voice: Option<String>`, and leave the bot crate with no native ML dependency and no `cmake`.

**Architecture:** The inference server gains `kokoro.rs` (engine + provisioning from the sherpa-onnx release archive), `opus.rs` (Ogg Opus encoder, moved), and `routes/speech.rs`. That route returns raw Ogg Opus plus two extension headers, `X-Duration-Ms` and `X-Waveform`. `backend/src/services/tts` is a client that posts in either dialect. It reads the extension headers when present, and otherwise falls back to the Ogg granule position for the duration, with no waveform. `modules/tts` keeps the business half: text preparation, per-user voice resolution, and the Matrix upload.

**Tech Stack:** Rust (Axum 0.7, reqwest 0.11, sherpa-onnx 1.13.8 `shared`, `opus` 0.4, `ogg` 0.9, `tar` 0.4, `bzip2` 0.5), SQLx over SQLite/PostgreSQL, React 19 + TypeScript.

**Spec:** `docs/superpowers/specs/2026-09-22-inference-service-design.md`. This plan implements phases 3 and 3a, and the TTS part of phase 5 (delivery).

**Branch:** `refactor/inference-tts`, stacked on `refactor/inference-stt`. Push to the `gitea` remote.

## Global Constraints

- The bot crate (`backend/`) must end with **no** `sherpa-onnx`, `opus`, `ort`, `ort-sys`, `image`, `paddle-ocr-rs`, `symphonia` or `opus-decoder` dependency. `ogg` stays (pure Rust; used for the granule fallback).
- `cargo check` must be warning-free for both crates (`cargo check --workspace --all-targets`).
- Nothing in `backend/src/services/` may import from `crate::modules` or `crate::config`.
- **No backward compatibility** (CLAUDE.md): the integer speaker path is removed, not kept alongside. No data migration from `tts_speaker_id` to `tts_voice`. `TTS_ENABLED`, `TTS_SPEAKER_ID` and the bot's `TTS_MODEL_DIR` are deleted, not deprecated.
- Wire shapes shared by the two crates are pinned by JSON fixtures in `contracts/`, and **both** crates' tests read the same fixture.
- User-facing error replies never echo a raw upstream error body or URL. Log them with `warn!`, and reply with a fixed message (the same rule `modules/voice` follows).
- Never log or `{:?}`-print an API key. Clients implement `Debug` by hand with the key redacted.
- Model downloads go to a temporary name and are renamed into place, so a route never sees a partial model.
- Commit after every task using conventional commits, ending with:
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`
- Windows caveat for anything that **loads** a sherpa model: a mismatched `onnxruntime.dll` kills the test process with an access violation. Such tests are `#[ignore]`d.
- Windows caveat for running the inference test binary at all: sherpa's `onnxruntime.dll` must sit beside it. If `cargo test -p matrix-bot-inference` dies at startup with `STATUS_ACCESS_VIOLATION` or `STATUS_DLL_NOT_FOUND`, run `cp target/debug/onnxruntime.dll target/debug/deps/` and re-run.

## Decisions this plan makes that the spec left open (or changed)

1. **Base URL is the on/off switch; `TTS_ENABLED` goes.** It existed to stop a 311 MB model loading in the bot, and the bot no longer loads models. OCR and STT degrade on an absent base URL, and TTS now does the same. Compose defaults `TTS_BASE_URL` to the bundled server, and an explicitly empty value switches TTS off.
2. **`text.rs` stays in `modules/tts`.** It strips HTML and emoji from a Matrix message. That is bot business, and it improves the output of *every* provider, not just Kokoro. The spec listed it as moving; the route instead takes plain text, as OpenAI's does.
3. **Kokoro is provisioned automatically**, like OCR and STT: `kokoro-multi-lang-v1_1.tar.bz2` (365 MB) is downloaded from the sherpa-onnx `tts-models` release at inference startup. It is extracted with `tar` + `bzip2` into a staging directory, then renamed into place.
4. **The bundled server's voices are Kokoro speaker ids as strings** (`"0"` to `"102"`). A non-integer voice is a 400, and an out-of-range id is clamped, as `clamp_speaker_id` already did. `TTS_VOICE` defaults to `"0"` and `TTS_MODEL` to `"kokoro"` (ignored by the bundled server).
5. **`backend/examples/tts_spike.rs` is deleted, not moved.** Its own header says to delete it once its question was answered, and it was. The external check it provided (`ffprobe` over a real `.ogg`) is replaced by the acceptance step at the end of this plan.
6. **ElevenLabs output format is `opus_48000_64`**, passed as the `output_format` query parameter. ElevenLabs returns that format as Ogg Opus, so the granule fallback applies.
7. **Voice validation is shape-only**: 1 to 64 chars of `[A-Za-z0-9_-]`. That admits Kokoro ids, OpenAI names and ElevenLabs ids, and rejects anything that could not be a path segment or a JSON string of a voice name. A voice the provider does not know fails at synthesis time with the provider's own 4xx.

---

### Task 1: Extract the streaming download helper into `inference/src/download.rs`

Kokoro provisioning (Task 2) needs the same idle-timeout streaming download SenseVoice uses. It lives privately in `sherpa.rs` with a `VoiceError` return type. Move it into its own module with its own error type. This is behaviour-preserving.

**Files:**
- Create: `inference/src/download.rs`
- Create: `inference/src/test_support.rs`
- Modify: `inference/src/sherpa.rs` (remove `DOWNLOAD_IDLE_TIMEOUT`, `download_model_with_idle`, `remove_partial_download`, `stream_to_file`, and the two `test_download_model_with_idle_*` tests plus its private `spawn_server`/`scratch_dest` test helpers; `download_model` becomes a one-line wrapper)
- Modify: `inference/src/main.rs` (add `mod download;` and `#[cfg(test)] mod test_support;`)

**Interfaces:**
- Produces: `crate::download::download_to(url: &str, dest: &Path) -> Result<(), DownloadError>`, `crate::download::DownloadError(pub String)` (implements `Display` via thiserror), and `crate::test_support::spawn_server(app: axum::Router) -> String` (test-only).

- [ ] **Step 1: Create `inference/src/test_support.rs`**

```rust
//! Helpers shared by this crate's tests.

/// Starts a throwaway server on loopback and returns its base URL, e.g.
/// `http://127.0.0.1:54321`. Uses axum and tokio, which this crate already
/// depends on, rather than a mock-HTTP crate.
pub(crate) async fn spawn_server(app: axum::Router) -> String {
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
```

- [ ] **Step 2: Create `inference/src/download.rs` with the moved code and tests**

Move the bodies of `download_model_with_idle`, `remove_partial_download` and `stream_to_file` from `sherpa.rs` verbatim, changing only the error type: every `VoiceError::ModelDownload(msg)` becomes `DownloadError(msg)`. Final file:

```rust
//! Streaming model downloads, shared by every capability's provisioning.

use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tracing::warn;

/// How long to wait for the next chunk of a model download before giving up.
/// This bounds idle time, not total transfer time: a slow-but-steady
/// multi-hundred-megabyte download on a modest link is not a stall, but a
/// connection with no bytes arriving for this long is.
const DOWNLOAD_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// Why a download failed, as a message for the caller's own error type.
#[derive(Debug, Error)]
#[error("{0}")]
pub struct DownloadError(pub String);

/// Streams `url` into `dest` with the default idle timeout.
pub async fn download_to(url: &str, dest: &Path) -> Result<(), DownloadError> {
    download_with_idle(url, dest, DOWNLOAD_IDLE_TIMEOUT).await
}

/// Streams `url` into `dest` via a `.tmp` file that is renamed into place
/// only on success, so anything watching `dest` never sees a partial file.
/// There is deliberately no total-transfer timeout, only `idle`, applied to
/// each chunk: the models are hundreds of megabytes, and a total timeout
/// fails deterministically on any link slower than it assumes.
async fn download_with_idle(url: &str, dest: &Path, idle: Duration) -> Result<(), DownloadError> {
    // ...body moved from sherpa.rs::download_model_with_idle, with
    // VoiceError::ModelDownload(x) -> DownloadError(x)
}

// remove_partial_download and stream_to_file: moved verbatim, same error change.

#[cfg(test)]
mod tests {
    // The two tests moved from sherpa.rs, renamed:
    //   test_download_with_idle_streams_a_full_body_to_disk
    //   test_download_with_idle_fails_and_cleans_up_on_a_stalled_stream
    // They call `download_with_idle`, use `crate::test_support::spawn_server`,
    // and a local `scratch_dest(tag)` helper (moved from sherpa.rs, file prefix
    // "download-" instead of "sherpa-download-"). The stalled-stream test now
    // asserts `result.is_err()` in place of `matches!(.., Err(VoiceError::ModelDownload(_)))`.
}
```

The two comment placeholders above name code that already exists in `sherpa.rs`. Cut it from there and paste it here. Do not rewrite it.

- [ ] **Step 3: Point `sherpa.rs` at the new module**

Replace `sherpa.rs::download_model` with:

```rust
async fn download_model(url: &str, dest: &Path) -> Result<(), VoiceError> {
    crate::download::download_to(url, dest)
        .await
        .map_err(|e| VoiceError::ModelDownload(e.0))
}
```

Remove the now-unused imports (`tokio::io::AsyncWriteExt`, and `Duration` or `warn` if nothing else in `sherpa.rs` uses them). Add `mod download;` to `main.rs` next to the other `mod` lines, and `#[cfg(test)] mod test_support;`.

- [ ] **Step 4: Verify**

Run: `cargo test -p matrix-bot-inference download`
Expected: 2 passed.
Run: `cargo check -p matrix-bot-inference --all-targets`
Expected: no warnings.
Run: `cargo test -p matrix-bot-inference`
Expected: all pass; ignored tests stay ignored.

- [ ] **Step 5: Commit**

```bash
git add inference/src/download.rs inference/src/test_support.rs inference/src/sherpa.rs inference/src/main.rs
git commit -m "refactor: share the streaming model download across inference capabilities"
```

---

### Task 2: Kokoro provisioning, `TTS_MODEL_DIR`, and `/health` reporting TTS

**Files:**
- Create: `inference/src/kokoro.rs`
- Modify: `inference/Cargo.toml` (add `tar = "0.4"`, `bzip2 = "0.5"`)
- Modify: `inference/src/config.rs` (add `tts_model_dir`)
- Modify: `inference/src/provision.rs` (provision TTS concurrently)
- Modify: `inference/src/routes/health.rs` (`tts` from `kokoro::model_present`)
- Modify: `inference/src/main.rs` (`mod kokoro;`)
- Modify: every `InferenceConfig { .. }` struct literal in tests (`grep -rn "InferenceConfig {" inference/src`) to add a `tts_model_dir`
- Modify: `Dockerfile.inference` (ENV `TTS_MODEL_DIR`), `docker-compose.yml` (inference service env `TTS_MODEL_DIR`)

**Interfaces:**
- Consumes: `crate::download::download_to` (Task 1).
- Produces: `crate::kokoro::{model_present(dir: &Path) -> bool, ensure_model_downloaded(dir: &Path) -> Result<(), TtsError>, TtsError, KOKORO_REQUIRED_FILES}`, with `TtsError::ModelDownload(String)`. Task 3 adds `ModelMissing(PathBuf)`, `Engine(String)` and `Encode(String)`. Also produces `InferenceConfig::tts_model_dir: PathBuf`.

- [ ] **Step 1: Add dependencies**

In `inference/Cargo.toml` `[dependencies]`, add:

```toml
tar = "0.4"
bzip2 = "0.5"
```

(`Cargo.lock` already carries `bzip2 0.5.2` and `tar 0.4.46` transitively, so no new download is expected.)

- [ ] **Step 2: Write the failing tests in `inference/src/kokoro.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kokoro-{}-{}", tag, std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A `.tar.bz2` shaped like the sherpa-onnx release: every entry under
    /// `KOKORO_ARCHIVE_ROOT/`.
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
        format!("{}/kokoro.tar.bz2", crate::test_support::spawn_server(app).await)
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
        // Port 1 refuses connections: reaching the network would fail the test.
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
        assert!(!root.join(ARCHIVE_FILE_NAME).exists(), "archive must be removed");
        assert!(!root.join(STAGING_DIR_NAME).exists(), "staging must be removed");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn test_an_archive_missing_required_files_installs_nothing() {
        let root = scratch("incomplete");
        let model_dir = root.join("kokoro");
        let url = serve_archive(archive_with(&["model.onnx"])).await;

        let result = ensure_model_from(&url, &model_dir).await;

        assert!(matches!(result, Err(TtsError::ModelDownload(_))), "{:?}", result);
        assert!(!model_dir.exists());
        assert!(!root.join(STAGING_DIR_NAME).exists());
        assert!(!root.join(ARCHIVE_FILE_NAME).exists());
        std::fs::remove_dir_all(&root).ok();
    }

    /// A directory someone put there by hand is never deleted to make room.
    #[tokio::test]
    async fn test_an_incomplete_non_empty_model_dir_is_left_alone() {
        let root = scratch("occupied");
        let model_dir = root.join("kokoro");
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("notes.txt"), b"mine").unwrap();
        let url = serve_archive(archive_with(&KOKORO_REQUIRED_FILES)).await;

        let result = ensure_model_from(&url, &model_dir).await;

        assert!(matches!(result, Err(TtsError::ModelDownload(_))), "{:?}", result);
        assert_eq!(std::fs::read(model_dir.join("notes.txt")).unwrap(), b"mine");
        std::fs::remove_dir_all(&root).ok();
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p matrix-bot-inference kokoro`
Expected: compile error, because `kokoro` items are not defined yet.

- [ ] **Step 4: Implement `inference/src/kokoro.rs` (provisioning half)**

Put this above the test module:

```rust
//! Kokoro text-to-speech: the model on disk, and (Task 3) the engine.

use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::info;

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

/// Task 3 adds `ModelMissing`, `Engine` and `Encode` when the engine that
/// constructs them arrives; adding them now would be dead code.
#[derive(Debug, Error)]
pub enum TtsError {
    #[error("Failed to install the Kokoro TTS model: {0}")]
    ModelDownload(String),
}

/// Whether every required model file is on disk. The route gates on this and
/// answers 503 until provisioning has finished.
pub fn model_present(dir: &Path) -> bool {
    KOKORO_REQUIRED_FILES.iter().all(|name| dir.join(name).is_file())
}

/// Installs the Kokoro model into `model_dir` unless it is already there.
pub async fn ensure_model_downloaded(model_dir: &Path) -> Result<(), TtsError> {
    ensure_model_from(KOKORO_ARCHIVE_URL, model_dir).await
}

/// Downloads the archive at `url` beside `model_dir`, unpacks it into a
/// staging directory, and renames the archive's root into place, so the route
/// sees either no model or a whole one. The archive and staging directory are
/// removed whether or not installation succeeds.
async fn ensure_model_from(url: &str, model_dir: &Path) -> Result<(), TtsError> {
    if model_present(model_dir) {
        return Ok(());
    }

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
            tokio::task::spawn_blocking(move || install_from_archive(&archive, &staging, &model_dir))
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

/// Unpacks `archive` into `staging` and moves its root to `model_dir`.
///
/// An existing `model_dir` is replaced only when it is empty. A non-empty
/// one that lacks the model is someone's hand-placed directory, and deleting
/// it to make room is not this function's call.
fn install_from_archive(archive: &Path, staging: &Path, model_dir: &Path) -> Result<(), TtsError> {
    if staging.exists() {
        std::fs::remove_dir_all(staging).map_err(|e| install_error("clear the staging directory", e))?;
    }
    std::fs::create_dir_all(staging).map_err(|e| install_error("create the staging directory", e))?;

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

    if model_dir.exists() {
        let empty = std::fs::read_dir(model_dir)
            .map_err(|e| install_error("inspect the model directory", e))?
            .next()
            .is_none();
        if !empty {
            return Err(TtsError::ModelDownload(format!(
                "{:?} exists but does not hold the model; remove it or point TTS_MODEL_DIR elsewhere",
                model_dir
            )));
        }
        std::fs::remove_dir(model_dir).map_err(|e| install_error("replace the empty model directory", e))?;
    }

    std::fs::rename(&root, model_dir).map_err(|e| install_error("move the model into place", e))
}

fn install_error(action: &str, err: std::io::Error) -> TtsError {
    TtsError::ModelDownload(format!("Failed to {}: {}", action, err))
}
```

Add `mod kokoro;` to `main.rs`.

- [ ] **Step 5: Run the kokoro tests**

Run: `cargo test -p matrix-bot-inference kokoro`
Expected: 5 passed.

- [ ] **Step 6: Add `tts_model_dir` to `InferenceConfig`**

In `inference/src/config.rs`, add the field `pub tts_model_dir: PathBuf`, with default `PathBuf::from("data/tts_models/kokoro-multi-lang-v1_1")`, read from `TTS_MODEL_DIR` exactly as `voice_model_dir` reads `VOICE_MODEL_DIR`. Then fix every test literal that `grep -rn "InferenceConfig {" inference/src` finds by adding `tts_model_dir: std::path::PathBuf::from("/nonexistent/tts/models"),`. The exception is `provision.rs`'s test, which uses `dir.clone()` (Step 7).

- [ ] **Step 7: Provision TTS beside OCR and STT**

In `provision.rs::provision_models`, add a third future and join all three:

```rust
    let tts_config = Arc::clone(&config);
    let tts = async move {
        retry_until_ready("Text-to-speech", || {
            crate::kokoro::ensure_model_downloaded(&tts_config.tts_model_dir)
        })
        .await;
    };

    tokio::join!(ocr, stt, tts);
```

Update the doc comment's "OCR and STT provision concurrently" to name all three. In the test `provisioned_dir`, add `crate::kokoro::KOKORO_REQUIRED_FILES` to the files written (iterate it after the existing array), and set `tts_model_dir: dir.clone()` in the test's `InferenceConfig`.

- [ ] **Step 8: Report TTS in `/health`**

In `routes/health.rs`, replace `tts: false` with `tts: crate::kokoro::model_present(&state.config.tts_model_dir)`, and replace the doc sentence "`tts` alone is hardcoded false until that capability lands; see the TTS plan." with "Each flag is that capability's `models_present` check." The two existing tests keep expecting `tts: false`, because their dirs are `/nonexistent/...`.

- [ ] **Step 9: Deployment config**

`Dockerfile.inference`: extend the `ENV` block with `TTS_MODEL_DIR=/app/data/tts_models/kokoro-multi-lang-v1_1`.
`docker-compose.yml`, `inference` service `environment`: add `- TTS_MODEL_DIR=/app/data/tts_models/kokoro-multi-lang-v1_1`.

- [ ] **Step 10: Verify**

Run: `cargo test -p matrix-bot-inference`, then `cargo check -p matrix-bot-inference --all-targets`.
Expected: all pass, and no warnings. Resolve any dead-code warning by narrowing visibility or deferring the item to Task 3, never with `allow(dead_code)`.

- [ ] **Step 11: Commit**

```bash
git add inference Dockerfile.inference docker-compose.yml Cargo.lock
git commit -m "feat(inference): provision the Kokoro TTS model and report it in /health"
```

---

### Task 3: Kokoro synthesis, the Ogg Opus encoder, and `POST /v1/audio/speech`

**Files:**
- Modify: `inference/src/kokoro.rs` (add the engine, moved from `backend/src/modules/tts/engine.rs`)
- Create: `inference/src/opus.rs` (moved from `backend/src/modules/tts/opus.rs`)
- Create: `inference/src/routes/speech.rs`
- Create: `contracts/tts_speech_request.json`, `contracts/tts_speech_headers.json`
- Modify: `inference/src/routes/mod.rs` (route), `inference/src/main.rs` (`mod opus;`), `inference/Cargo.toml` (`opus = "0.4"`)
- Modify: `Dockerfile.inference` (build stage installs `cmake`), `.github/workflows/ci.yml` (`inference` job installs `cmake`)

The backend copies of `engine.rs` and `opus.rs` stay until Task 6. Both crates carry them for three commits, which is expected.

**Interfaces:**
- Consumes: `kokoro::{model_present, TtsError, KOKORO_REQUIRED_FILES}` (Task 2).
- Produces: the HTTP contract `POST /v1/audio/speech`, JSON `{ model?, input, voice, response_format? }` → `200` with `content-type: audio/ogg`, `x-duration-ms: <u64>`, `x-waveform: <100 comma-separated u16 in 0..=1024>`, and a raw Ogg Opus body. Errors: `400` (bad input, voice, or format), `422` (missing/ill-typed JSON field; axum's own), `503` (model not installed), `500` (engine or encoding). Also the fixtures `contracts/tts_speech_request.json` and `contracts/tts_speech_headers.json`, which Task 4 reads.

- [ ] **Step 1: Create the contract fixtures**

`contracts/tts_speech_request.json`:

```json
{
  "model": "kokoro",
  "input": "早安，今天有三件待办事项。",
  "voice": "0",
  "response_format": "opus"
}
```

`contracts/tts_speech_headers.json`:

```json
{
  "x-duration-ms": "1500",
  "x-waveform": "0,512,1024"
}
```

- [ ] **Step 2: Move the encoder**

Add `opus = "0.4"` to `inference/Cargo.toml`. Copy `backend/src/modules/tts/opus.rs` to `inference/src/opus.rs` unchanged, except for its module doc. Replace the doc's third paragraph ("Uses the `opus` crate … no new dependencies.") with:

```rust
//! Uses the `opus` crate (libopus, built from source by `opusic-sys`, which
//! is why this crate's build needs `cmake`) for encoding and
//! `ogg::PacketWriter` for the container.
```

Add `mod opus;` to `main.rs`.

- [ ] **Step 3: Install cmake where the inference crate builds**

`Dockerfile.inference`, build stage: change the apt line to `pkg-config libssl-dev cmake`, with this comment above the `RUN`:

```dockerfile
# cmake builds the vendored libopus inside opusic-sys (the Ogg Opus encoder
# for text-to-speech). It links statically, so the runtime stage needs no
# libopus package.
```

`.github/workflows/ci.yml`, `inference` job, "Install Linux build dependencies": change the apt list to `pkg-config libssl-dev cmake`. Replace the sentence "Unlike the backend there is no cmake here -- nothing in this crate's graph builds with it, so it is not installed." with "cmake builds the vendored libopus inside opusic-sys, which the text-to-speech route encodes Ogg Opus with."

- [ ] **Step 4: Move the engine into `kokoro.rs`**

From `backend/src/modules/tts/engine.rs`, move into `inference/src/kokoro.rs`: `NoCallback`, `RULE_FSTS`, the `ENGINE` static and its doc comment, `opt`, `build_config`, `engine`, `clamp_speaker_id`, `synthesize_sync`, and the tests `test_opt_returns_none_for_a_missing_file`, the four `test_clamp_speaker_id_*`, and the `#[ignore]`d real-model test. Adapt them as follows:

- Drop that file's `model_present`. Task 2's version (in `kokoro.rs` already) replaces it.
- Add a `Speech` type (moved from `backend/src/modules/tts/models.rs`, with its two duration tests):

```rust
/// Synthesised audio: raw samples plus the sample rate they were generated at.
#[derive(Debug, Clone)]
pub struct Speech {
    pub samples: Vec<f32>,
    pub sample_rate: i32,
}

impl Speech {
    /// The clip's length, derived from sample count and rate rather than
    /// stored separately, so the two can never disagree.
    pub fn duration(&self) -> std::time::Duration {
        if self.sample_rate <= 0 {
            return std::time::Duration::ZERO;
        }
        std::time::Duration::from_secs_f64(self.samples.len() as f64 / self.sample_rate as f64)
    }
}
```

- Extend `TtsError` with the variants the engine and route need:

```rust
    #[error("Kokoro TTS model is not installed in {0:?}")]
    ModelMissing(PathBuf),
    #[error("TTS engine error: {0}")]
    Engine(String),
    #[error("Ogg Opus encoding failed: {0}")]
    Encode(String),
```

- Replace `synthesize` (which took `&TtsConfig`) with:

```rust
/// Synthesises `text` with the Kokoro model in `model_dir` at `speaker_id`,
/// clamped to the voices the loaded model has.
///
/// Runs on a blocking worker thread: inference is synchronous and CPU-bound
/// for seconds at a time, and on a runtime thread it would stall every other
/// request this server is handling.
pub async fn synthesize(model_dir: PathBuf, text: String, speaker_id: i64) -> Result<Speech, TtsError> {
    tokio::task::spawn_blocking(move || synthesize_sync(&model_dir, speaker_id, &text))
        .await
        .map_err(|e| TtsError::Engine(format!("Worker thread panicked: {}", e)))?
}
```

- Rewrite the `ENGINE` doc comment's sentence about `inference/src/sherpa.rs` so it reads correctly from inside this crate: "`sherpa.rs::run_transcription_sync` rebuilds its SenseVoice recogniser on every call, which is fine there because that model loads in milliseconds. This model is 311 MB and takes seconds to load, so it is built once." Keep the rest, including the `Send + Sync` soundness note.
- The `#[ignore]`d real-model test now reads the model from `Path::new(env!("CARGO_MANIFEST_DIR")).join("../data/tts_models/kokoro-multi-lang-v1_1")`, returns early with an `eprintln!` skip message when `!model_present`, and calls `synthesize(dir.clone(), "Good morning, this is a test.".to_string(), 0)` and then the Chinese line with `0`. Keep its assertions and its Windows DLL doc comment. Change the doc's last paragraph to say the Dockerfile in question is `Dockerfile.inference`.

- [ ] **Step 5: Write the failing route tests in `inference/src/routes/speech.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;

    const REQUEST_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_request.json"
    ));
    const HEADERS_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_headers.json"
    ));

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
                voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
                tts_model_dir: std::path::PathBuf::from("/nonexistent/tts/models"),
            }),
        }
    }

    async fn post_speech(body: serde_json::Value) -> StatusCode {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/audio/speech")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        crate::routes::build_router(state_with_missing_models())
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    #[test]
    fn test_the_shared_request_contract_deserialises() {
        let request: SpeechRequest = serde_json::from_str(REQUEST_CONTRACT).unwrap();
        assert_eq!(request.model.as_deref(), Some("kokoro"));
        assert_eq!(request.voice, "0");
        assert_eq!(request.response_format.as_deref(), Some("opus"));
        assert_eq!(validate(&request).unwrap().1, 0);
    }

    #[test]
    fn test_speech_headers_match_the_shared_contract() {
        let headers = speech_headers(Duration::from_millis(1500), &[0, 512, 1024]);
        let expected: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(HEADERS_CONTRACT).unwrap();
        for (name, value) in expected {
            assert_eq!(
                headers.get(name.as_str()).and_then(|v| v.to_str().ok()),
                value.as_str(),
                "header {}",
                name
            );
        }
        assert_eq!(headers.get("content-type").unwrap(), "audio/ogg");
    }

    #[tokio::test]
    async fn test_a_valid_request_without_the_model_is_a_503() {
        let body = serde_json::from_str(REQUEST_CONTRACT).unwrap();
        assert_eq!(post_speech(body).await, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_input_errors_are_400s_even_without_the_model() {
        let long = "字".repeat(MAX_INPUT_CHARS + 1);
        let cases = [
            serde_json::json!({ "input": "   ", "voice": "0" }),
            serde_json::json!({ "input": long, "voice": "0" }),
            serde_json::json!({ "input": "hi", "voice": "alloy" }),
            serde_json::json!({ "input": "hi", "voice": "0", "response_format": "mp3" }),
        ];
        for body in cases {
            assert_eq!(post_speech(body.clone()).await, StatusCode::BAD_REQUEST, "{}", body);
        }
    }

    #[test]
    fn test_omitted_response_format_means_opus() {
        let request: SpeechRequest =
            serde_json::from_value(serde_json::json!({ "input": "hi", "voice": "3" })).unwrap();
        assert_eq!(validate(&request), Ok(("hi".to_string(), 3)));
    }

    #[test]
    fn test_status_for_every_tts_error() {
        let s = |e: TtsError| status_for(&e);
        assert_eq!(s(TtsError::ModelMissing("x".into())), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(s(TtsError::ModelDownload("x".into())), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(s(TtsError::Engine("x".into())), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(s(TtsError::Encode("x".into())), StatusCode::INTERNAL_SERVER_ERROR);
    }

    // Moved from backend/src/modules/tts/mod.rs, unchanged:
    // test_waveform_has_exactly_100_buckets_within_range,
    // test_silence_produces_an_all_zero_waveform,
    // test_waveform_on_empty_samples_is_all_zero,
    // test_waveform_handles_fewer_samples_than_buckets.
}
```

- [ ] **Step 6: Run the tests to see them fail**

Run: `cargo test -p matrix-bot-inference speech`
Expected: compile errors for `SpeechRequest`, `validate`, `speech_headers` and `status_for`.

- [ ] **Step 7: Implement `routes/speech.rs`**

```rust
use axum::{
    extract::State,
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::time::Duration;
use tracing::warn;

use super::AppState;
use crate::kokoro::TtsError;

/// OpenAI's own input ceiling for `/audio/speech`.
pub const MAX_INPUT_CHARS: usize = 4096;

/// Extension headers carrying what a Matrix voice message needs and raw
/// audio does not say. Third-party providers omit them, which is legal; the
/// bot then reads the duration from the Ogg stream and sends no waveform.
const DURATION_HEADER: &str = "x-duration-ms";
const WAVEFORM_HEADER: &str = "x-waveform";

/// How many buckets Matrix's `waveform` field expects for a voice message.
const WAVEFORM_BUCKETS: usize = 100;
/// The largest value a waveform bucket may hold, per the Matrix spec.
const WAVEFORM_MAX: f32 = 1024.0;

/// OpenAI's speech request. `model` (and any field OpenAI defines that is
/// not named here, such as `speed`) is accepted and ignored: this server has
/// one model.
#[derive(Debug, Deserialize)]
pub struct SpeechRequest {
    #[serde(default)]
    pub model: Option<String>,
    pub input: String,
    pub voice: String,
    #[serde(default)]
    pub response_format: Option<String>,
}

/// Checks the request before the model gate, so the caller's error is
/// reported whether or not the model is installed. Returns the trimmed input
/// and the Kokoro speaker id.
fn validate(request: &SpeechRequest) -> Result<(String, i64), String> {
    match request.response_format.as_deref() {
        None | Some("opus") => {}
        Some(other) => {
            return Err(format!(
                "Unsupported response_format '{}'; this server only produces opus",
                other
            ))
        }
    }

    let input = request.input.trim();
    if input.is_empty() {
        return Err("'input' is empty".to_string());
    }
    if input.chars().count() > MAX_INPUT_CHARS {
        return Err(format!("'input' exceeds {} characters", MAX_INPUT_CHARS));
    }

    let speaker_id = request.voice.trim().parse::<i64>().map_err(|_| {
        format!(
            "Unknown voice '{}'; this server's voices are Kokoro speaker ids (0, 1, 2, ...)",
            request.voice
        )
    })?;

    Ok((input.to_string(), speaker_id))
}

/// Exhaustive on purpose: a new `TtsError` variant must be a compile error
/// here, not a silent 500.
fn status_for(err: &TtsError) -> StatusCode {
    match err {
        TtsError::ModelMissing(_) | TtsError::ModelDownload(_) => StatusCode::SERVICE_UNAVAILABLE,
        TtsError::Engine(_) | TtsError::Encode(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Approximates a waveform for Matrix's voice-message `waveform` field ...
/// (move `compute_waveform` and its doc comment verbatim from
/// backend/src/modules/tts/mod.rs)
fn compute_waveform(samples: &[f32]) -> Vec<u16> { /* moved verbatim */ }

fn speech_headers(duration: Duration, waveform: &[u16]) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/ogg"));
    headers.insert(DURATION_HEADER, HeaderValue::from(duration.as_millis() as u64));
    let bars = waveform
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(",");
    if let Ok(value) = HeaderValue::from_str(&bars) {
        headers.insert(WAVEFORM_HEADER, value);
    }
    headers
}

/// `POST /v1/audio/speech`: OpenAI's text-to-speech endpoint, served by
/// Kokoro. The body is Ogg Opus, which is what makes a Matrix client draw a
/// voice bubble rather than a file card.
pub async fn speak(
    State(state): State<AppState>,
    Json(request): Json<SpeechRequest>,
) -> Result<Response, (StatusCode, String)> {
    let (input, speaker_id) = validate(&request).map_err(|m| (StatusCode::BAD_REQUEST, m))?;

    if !crate::kokoro::model_present(&state.config.tts_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "The text-to-speech model is not installed on this inference server".to_string(),
        ));
    }

    let fail = |err: TtsError| {
        warn!("Kokoro synthesis failed: {}", err);
        (status_for(&err), err.to_string())
    };

    let speech = crate::kokoro::synthesize(state.config.tts_model_dir.clone(), input, speaker_id)
        .await
        .map_err(fail)?;
    let ogg = crate::opus::encode_ogg_opus(&speech.samples, speech.sample_rate)
        .map_err(|e| fail(TtsError::Encode(e)))?;

    let headers = speech_headers(speech.duration(), &compute_waveform(&speech.samples));
    Ok((headers, ogg).into_response())
}
```

In `routes/mod.rs`, add `pub mod speech;` and `.route("/v1/audio/speech", post(speech::speak))`.

`cargo check` will flag any engine item that is now unused. The route uses `synthesize`, `Speech::duration` and `model_present`, so there should be none. If `KOKORO_REQUIRED_FILES` or anything else is still unused outside tests, make it private or remove it rather than adding `allow(dead_code)`.

- [ ] **Step 8: Verify**

Run: `cargo fmt --all` then `cargo test -p matrix-bot-inference`
Expected: all pass, including the moved opus and waveform tests.
Run: `cargo check -p matrix-bot-inference --all-targets`
Expected: no warnings.

- [ ] **Step 9: Commit**

```bash
git add inference contracts Dockerfile.inference .github/workflows/ci.yml Cargo.lock
git commit -m "feat(inference): serve Kokoro text-to-speech at POST /v1/audio/speech"
```

---

### Task 4: `services/tts`, an OpenAI-compatible client with the Ogg duration fallback

**Files:**
- Create: `backend/src/services/tts/mod.rs`
- Modify: `backend/src/services/mod.rs` (`pub mod tts;`)

**Interfaces:**
- Consumes: the fixtures `contracts/tts_speech_request.json` and `contracts/tts_speech_headers.json` (Task 3).
- Produces: `crate::services::tts::{TtsClient, SynthesizedSpeech, TtsServiceError, ogg_opus_duration}`:
  - `TtsClient::new(base_url: Option<String>, api_key: Option<String>, model: String) -> Self` (Task 5 adds a fourth parameter, `dialect`)
  - `TtsClient::synthesize(&self, text: &str, voice: &str) -> Result<SynthesizedSpeech, TtsServiceError>`
  - `pub struct SynthesizedSpeech { pub audio: Vec<u8>, pub duration: Option<Duration>, pub waveform: Option<Vec<u16>> }`
  - `pub enum TtsServiceError { NotConfigured, Unreachable(String), Upstream { status: u16, message: String } }`

- [ ] **Step 1: Write the failing tests** (bottom of the new file)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_support::spawn_server;
    use axum::{
        http::{HeaderMap as AxumHeaders, StatusCode},
        routing::post,
        Json, Router,
    };
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    const REQUEST_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_request.json"
    ));
    const HEADERS_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/tts_speech_headers.json"
    ));

    /// A minimal Ogg Opus stream: OpusHead with `pre_skip`, OpusTags, and two
    /// audio pages ending at `final_granule`. The packets are not real Opus;
    /// only the container is read.
    fn fake_ogg_opus(pre_skip: u16, final_granule: u64) -> Vec<u8> {
        use ogg::writing::{PacketWriteEndInfo, PacketWriter};
        let mut buf = Vec::new();
        {
            let mut writer = PacketWriter::new(Cursor::new(&mut buf));
            let mut head = b"OpusHead".to_vec();
            head.push(1);
            head.push(1);
            head.extend_from_slice(&pre_skip.to_le_bytes());
            head.extend_from_slice(&24_000u32.to_le_bytes());
            head.extend_from_slice(&0i16.to_le_bytes());
            head.push(0);
            writer.write_packet(head, 7, PacketWriteEndInfo::EndPage, 0).unwrap();
            writer
                .write_packet(b"OpusTags\0\0\0\0\0\0\0\0".to_vec(), 7, PacketWriteEndInfo::EndPage, 0)
                .unwrap();
            writer
                .write_packet(vec![0u8; 8], 7, PacketWriteEndInfo::EndPage, final_granule / 2)
                .unwrap();
            writer
                .write_packet(vec![0u8; 8], 7, PacketWriteEndInfo::EndStream, final_granule)
                .unwrap();
        }
        buf
    }

    #[derive(Debug, Default)]
    struct Captured {
        authorization: Option<String>,
        body: serde_json::Value,
    }

    /// An OpenAI-shaped fake that records the request and answers with
    /// `audio` plus `extra_headers`. Returns the base URL including `/v1`.
    async fn fake_openai(
        captured: Arc<Mutex<Captured>>,
        audio: Vec<u8>,
        extra_headers: Vec<(String, String)>,
    ) -> String {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(move |headers: AxumHeaders, Json(body): Json<serde_json::Value>| {
                let captured = captured.clone();
                let audio = audio.clone();
                let extra_headers = extra_headers.clone();
                async move {
                    *captured.lock().unwrap() = Captured {
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned),
                        body,
                    };
                    let mut response = axum::response::Response::new(axum::body::Body::from(audio));
                    for (name, value) in extra_headers {
                        response.headers_mut().insert(
                            axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                            axum::http::HeaderValue::from_str(&value).unwrap(),
                        );
                    }
                    response
                }
            }),
        );
        format!("{}/v1", spawn_server(app).await)
    }

    fn contract_headers() -> Vec<(String, String)> {
        let map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(HEADERS_CONTRACT).unwrap();
        map.into_iter()
            .map(|(k, v)| (k, v.as_str().unwrap().to_string()))
            .collect()
    }

    #[tokio::test]
    async fn test_an_unconfigured_client_reports_not_configured() {
        let client = TtsClient::new(None, None, "kokoro".to_string());
        assert!(matches!(
            client.synthesize("hi", "0").await,
            Err(TtsServiceError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn test_sends_the_contract_body_and_reads_the_contract_headers() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = fake_openai(captured.clone(), b"OggS-audio".to_vec(), contract_headers()).await;
        let client = TtsClient::new(Some(base), None, "kokoro".to_string());
        let contract: serde_json::Value = serde_json::from_str(REQUEST_CONTRACT).unwrap();

        let speech = client
            .synthesize(
                contract["input"].as_str().unwrap(),
                contract["voice"].as_str().unwrap(),
            )
            .await
            .expect("synthesis should succeed");

        let seen = captured.lock().unwrap();
        assert_eq!(seen.body, contract);
        assert!(seen.authorization.is_none(), "no key means no Authorization header");
        assert_eq!(speech.audio, b"OggS-audio");
        assert_eq!(speech.duration, Some(Duration::from_millis(1500)));
        assert_eq!(speech.waveform, Some(vec![0, 512, 1024]));
    }

    /// The path every third-party provider takes: no extension headers.
    #[tokio::test]
    async fn test_without_extension_headers_the_duration_comes_from_the_ogg_granule() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let ogg = fake_ogg_opus(312, 2 * 48_000 + 312);
        let base = fake_openai(captured, ogg, Vec::new()).await;
        let client = TtsClient::new(Some(base), Some("sk-test".to_string()), "tts-1".to_string());

        let speech = client.synthesize("hi", "alloy").await.unwrap();

        assert_eq!(speech.duration, Some(Duration::from_secs(2)));
        assert_eq!(speech.waveform, None);
    }

    #[tokio::test]
    async fn test_sends_a_bearer_token_when_configured() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = fake_openai(captured.clone(), Vec::new(), Vec::new()).await;
        let client = TtsClient::new(Some(base), Some("sk-test".to_string()), "tts-1".to_string());

        client.synthesize("hi", "alloy").await.unwrap();

        assert_eq!(captured.lock().unwrap().authorization.as_deref(), Some("Bearer sk-test"));
    }

    #[test]
    fn test_ogg_opus_duration_subtracts_pre_skip_and_rejects_non_ogg() {
        assert_eq!(
            ogg_opus_duration(&fake_ogg_opus(312, 48_000 + 312)),
            Some(Duration::from_secs(1))
        );
        assert_eq!(ogg_opus_duration(b"ID3 an mp3, not ogg"), None);
        assert_eq!(ogg_opus_duration(&[]), None);
    }

    #[test]
    fn test_a_malformed_waveform_header_is_ignored_not_trusted() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(WAVEFORM_HEADER, "1,2,banana".parse().unwrap());
        assert_eq!(header_waveform(&headers), None);
        headers.insert(WAVEFORM_HEADER, "1,2,5000".parse().unwrap());
        assert_eq!(header_waveform(&headers), None, "above 1024 is out of spec");
        headers.insert(WAVEFORM_HEADER, "".parse().unwrap());
        assert_eq!(header_waveform(&headers), None);
    }

    #[tokio::test]
    async fn test_an_upstream_error_carries_its_status_and_message() {
        let app = Router::new().route(
            "/v1/audio/speech",
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "model is not installed") }),
        );
        let base = format!("{}/v1", spawn_server(app).await);
        let client = TtsClient::new(Some(base), None, "kokoro".to_string());

        match client.synthesize("hi", "0").await {
            Err(TtsServiceError::Upstream { status, message }) => {
                assert_eq!(status, 503);
                assert!(message.contains("not installed"));
            }
            other => panic!("expected an upstream error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_an_unreachable_service_is_not_a_panic() {
        let client = TtsClient::new(Some("http://127.0.0.1:1/v1".to_string()), None, "kokoro".to_string());
        assert!(matches!(
            client.synthesize("hi", "0").await,
            Err(TtsServiceError::Unreachable(_))
        ));
    }

    #[test]
    fn test_debug_output_never_contains_the_api_key() {
        let client = TtsClient::new(
            Some("https://api.openai.com/v1".to_string()),
            Some("sk-very-secret".to_string()),
            "tts-1".to_string(),
        );
        let debug = format!("{:?}", client);
        assert!(!debug.contains("sk-very-secret"), "leaked: {}", debug);
        assert!(debug.contains("redacted"));
    }
}
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `cargo test -p matrix-bot services::tts`
Expected: compile error, because the `tts` module does not exist yet.

- [ ] **Step 3: Implement `backend/src/services/tts/mod.rs`**

```rust
//! Client for text-to-speech providers.
//!
//! Speaks OpenAI's `POST {base}/audio/speech` and asks for Ogg Opus, the
//! format that makes a Matrix client draw a voice bubble. The bundled
//! inference server adds two headers (`x-duration-ms`, `x-waveform`) that a
//! voice message needs and raw audio does not carry. A provider that omits
//! them is still fine: the duration is read from the Ogg stream itself, and
//! the bubble is drawn without amplitude bars.

use reqwest::header::HeaderMap;
use serde::Serialize;
use std::fmt;
use std::io::Cursor;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// Bounds a hung connection. Kokoro on a CPU takes seconds for a sentence;
/// hosted providers take less.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

const DURATION_HEADER: &str = "x-duration-ms";
const WAVEFORM_HEADER: &str = "x-waveform";
/// The largest value a Matrix waveform bucket may hold.
const WAVEFORM_MAX: u16 = 1024;
/// Ogg Opus granule positions are always in 48 kHz units (RFC 7845 section 4).
const OPUS_GRANULE_RATE: u64 = 48_000;

/// Synthesised audio and the metadata a Matrix voice message wants.
#[derive(Debug, Clone, PartialEq)]
pub struct SynthesizedSpeech {
    pub audio: Vec<u8>,
    pub duration: Option<Duration>,
    pub waveform: Option<Vec<u16>>,
}

#[derive(Debug, Error)]
pub enum TtsServiceError {
    #[error("Text-to-speech service is not configured")]
    NotConfigured,
    #[error("Text-to-speech service is unreachable: {0}")]
    Unreachable(String),
    #[error("Text-to-speech service returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

#[derive(Serialize)]
struct OpenAiSpeechRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
    response_format: &'static str,
}

/// An absent `base_url` is a normal state: text-to-speech is not configured
/// in this deployment, and every call reports `NotConfigured` so the caller
/// can degrade that one feature.
#[derive(Clone)]
pub struct TtsClient {
    base_url: Option<String>,
    api_key: Option<String>,
    model: String,
    http: reqwest::Client,
}

/// Written by hand so the API key never reaches a `{:?}` log line.
impl fmt::Debug for TtsClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtsClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .finish()
    }
}

impl TtsClient {
    pub fn new(base_url: Option<String>, api_key: Option<String>, model: String) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());
        let api_key = api_key
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty());
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();
        Self { base_url, api_key, model, http }
    }

    /// Speaks `text` in `voice` and returns Ogg Opus audio.
    pub async fn synthesize(&self, text: &str, voice: &str) -> Result<SynthesizedSpeech, TtsServiceError> {
        let base = self.base_url.as_ref().ok_or(TtsServiceError::NotConfigured)?;

        let mut request = self
            .http
            .post(format!("{}/audio/speech", base))
            .json(&OpenAiSpeechRequest {
                model: &self.model,
                input: text,
                voice,
                response_format: "opus",
            });
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await.map_err(|e| {
            warn!("Text-to-speech request failed: {}", e);
            TtsServiceError::Unreachable(e.to_string())
        })?;

        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(TtsServiceError::Upstream { status: status.as_u16(), message });
        }

        let headers = response.headers().clone();
        let audio = response
            .bytes()
            .await
            .map_err(|e| TtsServiceError::Unreachable(format!("Failed to read the audio: {}", e)))?
            .to_vec();

        let duration = header_duration(&headers).or_else(|| ogg_opus_duration(&audio));
        let waveform = header_waveform(&headers);
        Ok(SynthesizedSpeech { audio, duration, waveform })
    }
}

fn header_duration(headers: &HeaderMap) -> Option<Duration> {
    headers
        .get(DURATION_HEADER)?
        .to_str()
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .map(Duration::from_millis)
}

/// The bundled server's waveform, or `None` if the header is absent or not a
/// list of in-range buckets. A malformed waveform is dropped, not sent: a
/// client renders a bubble without bars, but may reject an out-of-spec one.
fn header_waveform(headers: &HeaderMap) -> Option<Vec<u16>> {
    let raw = headers.get(WAVEFORM_HEADER)?.to_str().ok()?.trim();
    if raw.is_empty() {
        return None;
    }
    raw.split(',')
        .map(|bar| bar.trim().parse::<u16>().ok().filter(|v| *v <= WAVEFORM_MAX))
        .collect()
}

/// Reads an Ogg Opus stream's playable length from its container alone: the
/// last page's granule position, minus the `OpusHead` pre-skip, in 48 kHz
/// units. `None` for anything that is not Ogg Opus.
pub fn ogg_opus_duration(bytes: &[u8]) -> Option<Duration> {
    let mut reader = ogg::reading::PacketReader::new(Cursor::new(bytes));
    let mut pre_skip: Option<u64> = None;
    let mut last_granule: Option<u64> = None;

    while let Ok(Some(packet)) = reader.read_packet() {
        if pre_skip.is_none() {
            let data = &packet.data;
            if data.len() < 12 || &data[..8] != b"OpusHead" {
                return None;
            }
            pre_skip = Some(u64::from(u16::from_le_bytes([data[10], data[11]])));
        }
        last_granule = Some(packet.absgp_page());
    }

    let samples = last_granule?.checked_sub(pre_skip?)?;
    Some(Duration::from_millis(samples * 1000 / OPUS_GRANULE_RATE))
}
```

Add `pub mod tts;` to `backend/src/services/mod.rs`.

- [ ] **Step 4: Verify**

Run: `cargo fmt --all` then `cargo test -p matrix-bot services::tts`
Expected: 9 passed.
Run: `cargo check -p matrix-bot --all-targets`
Expected: no warnings.

- [ ] **Step 5: Commit**

```bash
git add backend/src/services
git commit -m "feat: add an OpenAI-compatible services/tts client with an Ogg duration fallback"
```

---

### Task 5: The ElevenLabs dialect

**Files:**
- Modify: `backend/src/services/tts/mod.rs`

**Interfaces:**
- Produces: `pub enum TtsDialect { OpenAi, ElevenLabs }` (derives `Debug, Clone, Copy, PartialEq, Eq, Default`, where `OpenAi` is the default) and `TtsDialect::parse(value: &str) -> Result<TtsDialect, String>`. `TtsClient::new` gains a fourth parameter, `dialect: TtsDialect`. Task 6 reads `TTS_DIALECT` through `TtsDialect::parse`.

- [ ] **Step 1: Update every existing `TtsClient::new(...)` call in the tests** to pass `TtsDialect::OpenAi` as the fourth argument.

- [ ] **Step 2: Write the failing dialect tests** (append to the test module)

```rust
    #[derive(Debug, Default, Clone)]
    struct Seen {
        path: String,
        query: Option<String>,
        authorization: Option<String>,
        xi_api_key: Option<String>,
        body: serde_json::Value,
    }

    /// Accepts any POST path and records what arrived.
    async fn recording_server(seen: Arc<Mutex<Seen>>) -> String {
        let app = Router::new().fallback(
            move |uri: axum::http::Uri, headers: AxumHeaders, Json(body): Json<serde_json::Value>| {
                let seen = seen.clone();
                async move {
                    let header = |name: &str| {
                        headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
                    };
                    *seen.lock().unwrap() = Seen {
                        path: uri.path().to_string(),
                        query: uri.query().map(str::to_owned),
                        authorization: header("authorization"),
                        xi_api_key: header("xi-api-key"),
                        body,
                    };
                    fake_ogg_opus(312, 48_000 + 312)
                }
            },
        );
        format!("{}/v1", spawn_server(app).await)
    }

    #[test]
    fn test_dialect_parse() {
        assert_eq!(TtsDialect::parse(""), Ok(TtsDialect::OpenAi));
        assert_eq!(TtsDialect::parse("openai"), Ok(TtsDialect::OpenAi));
        assert_eq!(TtsDialect::parse(" ElevenLabs "), Ok(TtsDialect::ElevenLabs));
        assert!(TtsDialect::parse("azure").is_err());
    }

    /// The spec's dialect test: one input, two shapes on the wire.
    #[tokio::test]
    async fn test_openai_and_elevenlabs_shape_the_same_input_differently() {
        let openai_seen = Arc::new(Mutex::new(Seen::default()));
        let openai = TtsClient::new(
            Some(recording_server(openai_seen.clone()).await),
            Some("k-1".to_string()),
            "tts-1".to_string(),
            TtsDialect::OpenAi,
        );
        openai.synthesize("hello", "voice123").await.unwrap();

        let eleven_seen = Arc::new(Mutex::new(Seen::default()));
        let eleven = TtsClient::new(
            Some(recording_server(eleven_seen.clone()).await),
            Some("k-1".to_string()),
            "eleven_multilingual_v2".to_string(),
            TtsDialect::ElevenLabs,
        );
        let speech = eleven.synthesize("hello", "voice123").await.unwrap();

        let o = openai_seen.lock().unwrap().clone();
        assert_eq!(o.path, "/v1/audio/speech");
        assert_eq!(o.authorization.as_deref(), Some("Bearer k-1"));
        assert_eq!(o.xi_api_key, None);
        assert_eq!(
            o.body,
            serde_json::json!({ "model": "tts-1", "input": "hello", "voice": "voice123", "response_format": "opus" })
        );

        let e = eleven_seen.lock().unwrap().clone();
        assert_eq!(e.path, "/v1/text-to-speech/voice123");
        assert_eq!(e.query.as_deref(), Some("output_format=opus_48000_64"));
        assert_eq!(e.xi_api_key.as_deref(), Some("k-1"));
        assert_eq!(e.authorization, None);
        assert_eq!(
            e.body,
            serde_json::json!({ "text": "hello", "model_id": "eleven_multilingual_v2" })
        );
        assert_eq!(speech.duration, Some(Duration::from_secs(1)), "Ogg fallback applies");
    }

    /// The voice is a path segment under ElevenLabs, so it must not be able
    /// to add segments of its own.
    #[tokio::test]
    async fn test_an_elevenlabs_voice_is_one_encoded_path_segment() {
        let seen = Arc::new(Mutex::new(Seen::default()));
        let client = TtsClient::new(
            Some(recording_server(seen.clone()).await),
            None,
            "m".to_string(),
            TtsDialect::ElevenLabs,
        );
        client.synthesize("hi", "a/../b").await.unwrap();
        assert_eq!(seen.lock().unwrap().path, "/v1/text-to-speech/a%2F..%2Fb");
    }
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `cargo test -p matrix-bot services::tts`
Expected: compile error, because `TtsDialect` is not defined.

- [ ] **Step 4: Implement the dialect**

Add:

```rust
/// How a provider wants a speech request shaped. Most providers speak
/// OpenAI's shape and need only a base URL. ElevenLabs differs on four axes
/// at once (auth header, URL, where the voice goes, body field names), which
/// a base URL cannot reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TtsDialect {
    #[default]
    OpenAi,
    ElevenLabs,
}

impl TtsDialect {
    /// Parses `TTS_DIALECT`. Blank means the default.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "openai" => Ok(Self::OpenAi),
            "elevenlabs" => Ok(Self::ElevenLabs),
            other => Err(format!(
                "Unknown TTS_DIALECT '{}'; expected openai or elevenlabs",
                other
            )),
        }
    }
}

/// ElevenLabs returns this as Ogg Opus, so the granule fallback applies.
const ELEVENLABS_OUTPUT_FORMAT: &str = "opus_48000_64";

#[derive(Serialize)]
struct ElevenLabsSpeechRequest<'a> {
    text: &'a str,
    model_id: &'a str,
}
```

Add a `dialect: TtsDialect` field to `TtsClient`, with a fourth `new` parameter, and include `.field("dialect", &self.dialect)` in `Debug`. Move request construction out of `synthesize` into:

```rust
    fn build_request(&self, base: &str, text: &str, voice: &str) -> Result<reqwest::RequestBuilder, TtsServiceError> {
        match self.dialect {
            TtsDialect::OpenAi => {
                let mut request = self
                    .http
                    .post(format!("{}/audio/speech", base))
                    .json(&OpenAiSpeechRequest {
                        model: &self.model,
                        input: text,
                        voice,
                        response_format: "opus",
                    });
                if let Some(key) = &self.api_key {
                    request = request.bearer_auth(key);
                }
                Ok(request)
            }
            TtsDialect::ElevenLabs => {
                let invalid = |detail: String| {
                    TtsServiceError::Unreachable(format!("Invalid TTS_BASE_URL {}: {}", base, detail))
                };
                let mut url = reqwest::Url::parse(base).map_err(|e| invalid(e.to_string()))?;
                url.path_segments_mut()
                    .map_err(|_| invalid("it cannot carry a path".to_string()))?
                    .push("text-to-speech")
                    .push(voice);
                url.query_pairs_mut()
                    .append_pair("output_format", ELEVENLABS_OUTPUT_FORMAT);

                let mut request = self.http.post(url).json(&ElevenLabsSpeechRequest {
                    text,
                    model_id: &self.model,
                });
                if let Some(key) = &self.api_key {
                    request = request.header("xi-api-key", key);
                }
                Ok(request)
            }
        }
    }
```

In `synthesize`, replace the inline construction with `let request = self.build_request(base, text, voice)?;`. Update the module doc's first paragraph to add: "`TtsDialect::ElevenLabs` shapes the same request the way ElevenLabs wants it."

- [ ] **Step 5: Verify**

Run: `cargo fmt --all` then `cargo test -p matrix-bot services::tts`
Expected: 12 passed.
Run: `cargo check -p matrix-bot --all-targets`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
git add backend/src/services/tts
git commit -m "feat: speak the ElevenLabs dialect in services/tts"
```

---

### Task 6: Rewire `!say` onto `services/tts`, and remove the bot's native ML dependencies

After this task, `backend/` no longer links sherpa-onnx or libopus, and its builds need no `cmake`.

**Files:**
- Rewrite: `backend/src/modules/tts/mod.rs`
- Delete: `backend/src/modules/tts/engine.rs`, `backend/src/modules/tts/opus.rs`, `backend/src/modules/tts/models.rs`, `backend/examples/tts_spike.rs` (and `backend/examples/` if it is then empty)
- Keep unchanged: `backend/src/modules/tts/text.rs`
- Modify: `backend/src/config.rs`, `backend/src/api/auth/types.rs` (test `Config` literal), `backend/src/bot/mod.rs`, `backend/src/modules/mod.rs` (two test constructions of `TtsModule`)
- Modify: `backend/Cargo.toml` (remove `sherpa-onnx` and `opus`)
- Modify: `Dockerfile`, `.github/workflows/ci.yml` (both backend jobs), `.github/workflows/release.yml`, `docker-compose.yml` (bot env), `.env.example` (TTS section)

**Interfaces:**
- Consumes: `services::tts::{TtsClient, TtsDialect, TtsServiceError, SynthesizedSpeech}` (Tasks 4–5), and `UserSettings::tts_speaker_id: Option<i64>`, which is still an integer until Task 7.
- Produces: `TtsModule::new(client: Arc<RwLock<Option<Client>>>, tts: TtsClient, default_voice: String)`; `Config` fields `tts_base_url: Option<String>`, `tts_api_key: Option<String>`, `tts_model: String`, `tts_voice: String` and `tts_dialect: TtsDialect`; and `Config::tts_client(&self) -> TtsClient`. `Config::tts_config`, `tts_enabled`, `tts_model_dir` and `tts_speaker_id` are removed.

- [ ] **Step 1: Write the new `modules/tts/mod.rs` tests first**

Replace the test module with:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::tts::TtsDialect;
    use std::sync::Mutex;

    fn module_with(tts: TtsClient, default_voice: &str) -> TtsModule {
        TtsModule::new(Arc::new(RwLock::new(None)), tts, default_voice.to_string())
    }

    fn unconfigured() -> TtsClient {
        TtsClient::new(None, None, "kokoro".to_string(), TtsDialect::OpenAi)
    }

    fn ctx_for(command: &str, raw_args: &str) -> CommandContext {
        CommandContext {
            room_id: "!room:a".to_string(),
            owner_id: "@alice:a".to_string(),
            sender_mxid: "@alice:a".to_string(),
            sender_name: "alice".to_string(),
            command: command.to_string(),
            raw_args: raw_args.to_string(),
            full_body: format!("!{} {}", command, raw_args),
            is_direct: true,
            is_mentioned: false,
            quoted: None,
            stage_reporter: None,
        }
    }

    async fn memory_db() -> Database {
        Database::connect("sqlite::memory:", 1).await.unwrap()
    }

    #[test]
    fn test_advertises_the_say_command() {
        let module = module_with(unconfigured(), "0");
        assert_eq!(module.name(), "Text-to-Speech");
        let commands = module.commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "say");
    }

    #[tokio::test]
    async fn test_ignores_commands_it_does_not_claim() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        assert!(module.handle_command(&ctx_for("todo", "list"), &db, None).await.is_none());
    }

    #[tokio::test]
    async fn test_an_unconfigured_service_degrades_with_a_message() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let reply = module.handle_command(&ctx_for("say", "hello there"), &db, None).await.unwrap();
        assert!(reply.contains("语音合成不可用"), "{}", reply);
    }

    #[tokio::test]
    async fn test_emoji_only_text_is_refused_before_any_request() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let reply = module.handle_command(&ctx_for("say", "✅🎉"), &db, None).await.unwrap();
        assert!(reply.contains("没有可朗读的文字"), "{}", reply);
    }

    #[tokio::test]
    async fn test_overlong_text_is_refused_before_any_request() {
        let module = module_with(unconfigured(), "0");
        let db = memory_db().await;
        let long = "字".repeat(MAX_SPEAK_CHARS + 1);
        let reply = module.handle_command(&ctx_for("say", &long), &db, None).await.unwrap();
        assert!(reply.contains("文字过长"), "{}", reply);
    }

    /// Synthesis succeeds against a fake provider; sending then fails only
    /// because no Matrix client is connected in tests. The request proves
    /// which voice was resolved.
    #[tokio::test]
    async fn test_a_user_without_a_saved_voice_is_spoken_in_the_house_default() {
        use axum::{routing::post, Json, Router};
        let voice_seen = Arc::new(Mutex::new(None::<String>));
        let seen = voice_seen.clone();
        let app = Router::new().route(
            "/v1/audio/speech",
            post(move |Json(body): Json<serde_json::Value>| {
                let seen = seen.clone();
                async move {
                    *seen.lock().unwrap() = body["voice"].as_str().map(str::to_owned);
                    Vec::<u8>::new()
                }
            }),
        );
        let base = format!("{}/v1", crate::services::test_support::spawn_server(app).await);
        let tts = TtsClient::new(Some(base), None, "kokoro".to_string(), TtsDialect::OpenAi);
        let module = module_with(tts, "7");
        let db = memory_db().await;

        let reply = module.handle_command(&ctx_for("say", "hello"), &db, None).await.unwrap();

        assert!(reply.contains("语音发送失败"), "{}", reply);
        assert_eq!(voice_seen.lock().unwrap().as_deref(), Some("7"));
    }

    #[test]
    fn test_failure_messages_never_echo_the_upstream_body() {
        let leaky = "http://inference:8090/v1/audio/speech sherpa-internal-detail";
        let cases = [
            (TtsServiceError::NotConfigured, "语音合成不可用"),
            (TtsServiceError::Unreachable(leaky.to_string()), "语音合成不可用"),
            (TtsServiceError::Upstream { status: 503, message: leaky.to_string() }, "语音合成不可用"),
            (TtsServiceError::Upstream { status: 400, message: leaky.to_string() }, "!settings voice none"),
            (TtsServiceError::Upstream { status: 422, message: leaky.to_string() }, "!settings voice none"),
        ];
        for (err, expected) in cases {
            let reply = failure_message(&err);
            assert!(reply.contains(expected), "{:?} -> {}", err, reply);
            assert!(!reply.contains("sherpa-internal-detail"), "leaked: {}", reply);
        }
    }

    #[test]
    fn test_resolve_voice_prefers_the_saved_choice() {
        assert_eq!(resolve_voice("5", None), "5");
        assert_eq!(resolve_voice("5", Some(77)), "77");
    }

    /// Speaker 0 is a real voice: an explicit 0 must win over a non-zero
    /// house default.
    #[test]
    fn test_resolve_voice_honours_an_explicit_zero() {
        assert_eq!(resolve_voice("44", Some(0)), "0");
    }
}
```

- [ ] **Step 2: Rewrite the module body**

```rust
//! Text-to-speech: `!say <text>` replies with a spoken voice message.
//!
//! The speaking happens elsewhere, through `services::tts`: the bundled
//! inference server's Kokoro, or any OpenAI-compatible or ElevenLabs
//! provider, chosen by config. This module is the bot's half: preparing
//! the text (`text.rs`), deciding whose voice speaks, and sending the result
//! into the room as a voice message.

use async_trait::async_trait;
use matrix_sdk::attachment::{AttachmentConfig, AttachmentInfo, BaseAudioInfo};
use matrix_sdk::ruma::{RoomId, UInt};
use matrix_sdk::Client;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, warn};

use crate::db::Database;
use crate::events::EventBroadcaster;
use crate::modules::settings::SettingsRepository;
use crate::modules::{BotModule, CommandContext, CommandHelp};
use crate::services::tts::{SynthesizedSpeech, TtsClient, TtsServiceError};
use crate::utils::html::html_escape;

pub mod text;

/// Matches the other modules' 500-character command-input ceiling.
pub const MAX_SPEAK_CHARS: usize = 500;

/// The voice `!say` speaks in: the sender's own choice, or the operator's
/// `TTS_VOICE` for someone who never chose.
fn resolve_voice(house_default: &str, saved: Option<i64>) -> String {
    saved
        .map(|id| id.to_string())
        .unwrap_or_else(|| house_default.to_string())
}

/// What the sender is told when synthesis fails. Fixed text: the upstream
/// body can carry internal URLs and engine detail, which is logged instead.
fn failure_message(err: &TtsServiceError) -> String {
    match err {
        TtsServiceError::Upstream { status: 400 | 404 | 422, .. } => "⚠️ <b>语音合成失败</b><br/>\
             • <b>原因:</b> 语音合成服务拒绝了这段文字或所选的声音<br/>\
             • <b>建议:</b> 发送 <code>!settings voice none</code> 恢复默认声音后重试。"
            .to_string(),
        TtsServiceError::NotConfigured => "⚠️ <b>语音合成不可用</b><br/>\
             • <b>原因:</b> 本部署未配置语音合成服务<br/>\
             • <b>建议:</b> 请联系管理员设置 <code>TTS_BASE_URL</code>。"
            .to_string(),
        TtsServiceError::Unreachable(_) | TtsServiceError::Upstream { .. } => "⚠️ <b>语音合成不可用</b><br/>\
             • <b>原因:</b> 语音合成服务当前不可用<br/>\
             • <b>建议:</b> 请稍后重试。"
            .to_string(),
    }
}

pub struct TtsModule {
    /// The shared handle to the bot's own Matrix client. `AiModule` and
    /// `AccountModule` reach Matrix the same way; see `modules/ai/mod.rs`.
    client: Arc<RwLock<Option<Client>>>,
    tts: TtsClient,
    default_voice: String,
}

impl TtsModule {
    pub fn new(client: Arc<RwLock<Option<Client>>>, tts: TtsClient, default_voice: String) -> Self {
        Self { client, tts, default_voice }
    }

    async fn handle_say(&self, ctx: &CommandContext, db: &Database) -> String {
        let text = text::speakable(ctx.raw_args.trim());
        if text.is_empty() {
            return "⚠️ <b>没有可朗读的文字</b><br/>\
                    • <b>用法:</b> <code>!say &lt;文字&gt;</code>"
                .to_string();
        }
        if text.chars().count() > MAX_SPEAK_CHARS {
            return format!(
                "⚠️ <b>文字过长</b><br/>\
                 • <b>原因:</b> 超过 {} 字符上限，请缩短后重试。",
                MAX_SPEAK_CHARS
            );
        }

        let settings = SettingsRepository::get_or_default(&db.pool, &ctx.owner_id).await;
        let voice = resolve_voice(&self.default_voice, settings.tts_speaker_id);

        let speech = match self.tts.synthesize(&text, &voice).await {
            Ok(speech) => speech,
            Err(err) => {
                warn!("TTS synthesis failed: {}", err);
                return failure_message(&err);
            }
        };

        match self.send_voice_message(&ctx.room_id, speech).await {
            Ok(()) => format!("🔊 <b>已朗读:</b> “{}”", html_escape(&text)),
            Err(err) => {
                error!("Failed to send TTS voice message: {}", err);
                format!(
                    "⚠️ <b>语音发送失败</b><br/>\
                     • <b>原因:</b> {}",
                    html_escape(&err)
                )
            }
        }
    }

    /// Uploads the Ogg Opus clip and sends it as a voice message.
    ///
    /// `AttachmentInfo::Voice` is what makes a Matrix client draw a voice
    /// bubble rather than a file card. The waveform is `None` when the
    /// provider sent none, and a client then draws the bubble without bars.
    async fn send_voice_message(&self, room_id: &str, speech: SynthesizedSpeech) -> Result<(), String> {
        let client = self.client.read().await.clone();
        let client = client.ok_or_else(|| "Matrix client is not connected".to_string())?;

        let parsed_room_id = <&RoomId>::try_from(room_id)
            .map_err(|_| format!("Invalid Matrix room ID: {}", room_id))?;
        let room = client
            .get_room(parsed_room_id)
            .ok_or_else(|| format!("Bot is not joined to room {}", room_id))?;

        let content_type = "audio/ogg"
            .parse::<mime_guess::mime::Mime>()
            .map_err(|e| format!("Invalid MIME type: {}", e))?;

        let info = AttachmentInfo::Voice {
            audio_info: BaseAudioInfo {
                duration: speech.duration,
                size: UInt::new(speech.audio.len() as u64),
            },
            waveform: speech.waveform,
        };

        room.send_attachment(
            "voice-message.ogg",
            &content_type,
            speech.audio,
            AttachmentConfig::new().info(info),
        )
        .await
        .map_err(|e| format!("Matrix upload/send failed: {:?}", e))?;

        Ok(())
    }
}
```

Keep the `BotModule` impl as it is (`name`, `commands`, `handle_command`). Delete `engine.rs`, `opus.rs` and `models.rs`, and drop their `pub mod` / `pub use` lines.

- [ ] **Step 3: Config**

In `backend/src/config.rs`:
- Add `const DEFAULT_TTS_MODEL: &str = "kokoro";` and `const DEFAULT_TTS_VOICE: &str = "0";` next to `DEFAULT_STT_MODEL`.
- Replace the three TTS fields with:

```rust
    /// Base URL of a text-to-speech API, including its version prefix
    /// (`http://inference:8090/v1`, `https://api.openai.com/v1`,
    /// `https://api.elevenlabs.io/v1`). Absent means `!say` is unavailable in
    /// this deployment.
    pub tts_base_url: Option<String>,
    /// API key for a hosted provider: a bearer token under the OpenAI
    /// dialect, `xi-api-key` under ElevenLabs. The bundled server needs none.
    pub tts_api_key: Option<String>,
    /// Model name sent with each request. The bundled server ignores it.
    pub tts_model: String,
    /// The house voice, for anyone who never chose one: a Kokoro speaker id
    /// for the bundled server, a name such as `alloy` for OpenAI, a voice id
    /// for ElevenLabs.
    pub tts_voice: String,
    /// How requests are shaped for the provider at `tts_base_url`.
    pub tts_dialect: crate::services::tts::TtsDialect,
```

- In `from_env`, replace the three TTS reads with reads of `TTS_BASE_URL`, `TTS_API_KEY`, `TTS_MODEL` (default `DEFAULT_TTS_MODEL`) and `TTS_VOICE` (default `DEFAULT_TTS_VOICE`), each shaped exactly like the `STT_*` reads above them (trim, empty → absent or default). Then:

```rust
        let tts_dialect = crate::services::tts::TtsDialect::parse(
            &env::var("TTS_DIALECT").unwrap_or_default(),
        )
        .map_err(|e| anyhow::anyhow!(e))?;
```

(If `from_env`'s `Result` alias is not `anyhow::Result`, convert into whatever error type it already returns. Refusing to boot on an unknown dialect is the point.)
- Replace `tts_config()` with:

```rust
    /// The text-to-speech client for this deployment.
    pub fn tts_client(&self) -> crate::services::tts::TtsClient {
        crate::services::tts::TtsClient::new(
            self.tts_base_url.clone(),
            self.tts_api_key.clone(),
            self.tts_model.clone(),
            self.tts_dialect,
        )
    }
```

- Update the two test `Config` literals in `config.rs` and the one in `api/auth/types.rs`, replacing the three old TTS fields with `tts_base_url: None, tts_api_key: None, tts_model: "kokoro".to_string(), tts_voice: "0".to_string(), tts_dialect: crate::services::tts::TtsDialect::OpenAi,`.

- [ ] **Step 4: Wire it**

`backend/src/bot/mod.rs`: `TtsModule::new(<client handle as today>, config.tts_client(), config.tts_voice.clone())`.
`backend/src/modules/mod.rs`, both test constructions: `TtsModule::new(Arc::new(RwLock::new(None)), crate::services::tts::TtsClient::new(None, None, "kokoro".to_string(), crate::services::tts::TtsDialect::OpenAi), "0".to_string())`. Also drop the `TtsConfig` import there.

- [ ] **Step 5: Remove the native dependencies**

`backend/Cargo.toml`: delete the `sherpa-onnx = ...` and `opus = ...` lines. Delete `backend/examples/tts_spike.rs`.

Run: `cargo tree -p matrix-bot -e normal | grep -E "sherpa|opus|ort-sys|paddle|symphonia"`
Expected: no output. (`ogg` is expected and allowed.)

- [ ] **Step 6: Build files**

`Dockerfile`, build stage: the apt line becomes `pkg-config libssl-dev`. Replace the comment above it with: "reqwest pulls in native-tls (openssl-sys is in Cargo.lock), so the build needs the OpenSSL headers even though sqlx itself is on rustls. No cmake: the bot links no native ML or codec library; those live in the inference image." Runtime stage: delete the two `COPY --from=build ... .so` lines and `RUN ldconfig`.

`.github/workflows/ci.yml`, **both** `backend` and `backend-postgres` jobs:
- In "Install Linux build dependencies", the apt list becomes `pkg-config libssl-dev`. In the comment, delete the sentence "cmake builds the vendored libopus inside opusic-sys;", and change "the build fails on a missing cmake" to "the build fails on missing OpenSSL headers".
- Delete the steps "Resolve the sherpa-onnx version", "Cache the sherpa-onnx prebuilt archive", "Fetch the sherpa-onnx archive if the cache missed" and "Clean sherpa-onnx prebuilt cache", together with the comment blocks above them. The `inference` job keeps all four.
- In `backend-postgres`'s `container:` comment, change "cmake is the only real addition" to "the OpenSSL headers are the only real addition".

`.github/workflows/release.yml`: the apt list becomes `pkg-config libssl-dev`. Replace the comment above it with "reqwest's native-tls needs the OpenSSL headers and pkg-config to find them. This job declares its build dependencies rather than inheriting them from the runner image."

`docker-compose.yml`, `matrix-bot` environment, after `VOICE_LANGUAGE`:

```yaml
      - TTS_BASE_URL=${TTS_BASE_URL-http://inference:8090/v1}
      - TTS_API_KEY=${TTS_API_KEY:-}
      - TTS_MODEL=${TTS_MODEL:-kokoro}
      - TTS_VOICE=${TTS_VOICE:-0}
      - TTS_DIALECT=${TTS_DIALECT:-openai}
```

`.env.example`: replace the whole "Text-to-Speech (sherpa-onnx Kokoro)" section with:

```bash
# --- Text-to-Speech (!say voice replies) --------------------------------------
# Base URL of a text-to-speech API, INCLUDING its /v1 prefix. Read by the bot.
# Leave commented out to run without !say (it then answers that speech is
# unavailable, and nothing else changes). Under `docker compose up` this
# defaults to the bundled inference server's Kokoro (http://inference:8090/v1);
# set it to an explicitly empty value (TTS_BASE_URL=) to switch !say off.
#
# Hosted providers -- set the URL, key, model and voice together:
#   OpenAI:     TTS_BASE_URL=https://api.openai.com/v1      TTS_MODEL=tts-1  TTS_VOICE=alloy
#   ElevenLabs: TTS_BASE_URL=https://api.elevenlabs.io/v1   TTS_DIALECT=elevenlabs
#               TTS_MODEL=eleven_multilingual_v2             TTS_VOICE=<voice id>
# TTS_BASE_URL=http://inference:8090/v1
# Bearer token (OpenAI dialect) or xi-api-key (ElevenLabs). The bundled server needs none.
# TTS_API_KEY=
# Model name sent with each request (default: kokoro, which the bundled server ignores).
# TTS_MODEL=kokoro
# House default voice for anyone who never chose one: a Kokoro speaker id
# (0-102) for the bundled server, otherwise a name or id your provider knows.
# Each user can override it with `!settings voice <voice>` or in the web
# Settings modal; `!settings voice none` returns them to this default.
# TTS_VOICE=0
# openai (default) or elevenlabs. The bot refuses to start on anything else.
# TTS_DIALECT=openai

# Read by the inference container, not the bot. Directory the Kokoro model
# (365 MB download) is unpacked into at inference-server startup.
TTS_MODEL_DIR=/app/data/tts_models/kokoro-multi-lang-v1_1
```

- [ ] **Step 7: Verify**

Run: `cargo fmt --all` then `cargo check --workspace --all-targets`
Expected: no warnings.
Run: `cargo test -p matrix-bot`
Expected: all pass.
Run: `cargo test -p matrix-bot modules::tts`
Expected: 9 passed.

- [ ] **Step 8: Commit**

```bash
git add -A backend Dockerfile .github docker-compose.yml .env.example Cargo.lock
git commit -m "refactor: speak !say through services/tts and drop the bot's native ML dependencies"
```

---

### Task 7: The per-user voice becomes `tts_voice: Option<String>`

**Files:**
- Modify: `backend/src/modules/settings/models.rs`, `repository.rs`, `handler.rs`, `mod.rs`
- Modify: `backend/src/db/schema_settings.rs`
- Modify: `backend/src/api/settings.rs`
- Modify: `backend/src/modules/tts/mod.rs` (`resolve_voice` takes `Option<&str>`)
- Regenerate: `backend/openapi.json`, `frontend/src/types/api.ts`
- Modify: `frontend/src/core/settings.ts`, `frontend/src/components/SettingsModal.tsx`

**Interfaces:**
- Produces: `UserSettings::tts_voice: Option<String>`, `modules::settings::models::{parse_tts_voice(raw: &str) -> Result<String, String>, MAX_TTS_VOICE_LEN: usize}`, and the API field `tts_voice: string | null`. `MAX_TTS_SPEAKER_ID` and `tts_speaker_id` are removed everywhere.

- [ ] **Step 1: Write the failing validation tests** in `modules/settings/models.rs`'s test module

```rust
    #[test]
    fn test_parse_tts_voice_accepts_every_provider_shape() {
        assert_eq!(parse_tts_voice("0"), Ok("0".to_string()));
        assert_eq!(parse_tts_voice(" alloy "), Ok("alloy".to_string()));
        assert_eq!(
            parse_tts_voice("21m00Tcm4TlvDq8ikWAM"),
            Ok("21m00Tcm4TlvDq8ikWAM".to_string())
        );
        assert_eq!(parse_tts_voice("zf_xiaobei-2"), Ok("zf_xiaobei-2".to_string()));
    }

    #[test]
    fn test_parse_tts_voice_rejects_anything_that_is_not_a_voice_name() {
        let too_long = "a".repeat(MAX_TTS_VOICE_LEN + 1);
        for bad in ["", "   ", "a b", "../x", "a/b", "é", "a?b", too_long.as_str()] {
            assert!(parse_tts_voice(bad).is_err(), "accepted {:?}", bad);
        }
    }
```

Run: `cargo test -p matrix-bot parse_tts_voice`
Expected: compile error.

- [ ] **Step 2: Implement in `models.rs`**

Replace `MAX_TTS_SPEAKER_ID` and its doc comment with:

```rust
/// The longest voice name or id stored. ElevenLabs ids are 20 characters and
/// OpenAI's names are shorter; this only refuses the absurd.
pub const MAX_TTS_VOICE_LEN: usize = 64;

/// Checks that `raw` could be a voice. What voices exist depends on the
/// configured provider, which the bot does not know. A Kokoro speaker id, an
/// OpenAI name and an ElevenLabs id all fit this shape, and a voice the
/// provider does not have fails at `!say` time with the provider's own error.
pub fn parse_tts_voice(raw: &str) -> Result<String, String> {
    let voice = raw.trim();
    let valid = !voice.is_empty()
        && voice.len() <= MAX_TTS_VOICE_LEN
        && voice
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if valid {
        Ok(voice.to_string())
    } else {
        Err(format!(
            "A voice is 1-{} letters, digits, '_' or '-' (e.g. 0, alloy, 21m00Tcm4TlvDq8ikWAM)",
            MAX_TTS_VOICE_LEN
        ))
    }
}
```

Replace the `tts_speaker_id` field (and its doc) with:

```rust
    /// The voice `!say` speaks in, in whatever the configured provider calls
    /// it. `None` means no preference, and `modules::tts` falls back to the
    /// operator's `TTS_VOICE`: the same "no row until you change something"
    /// convention `dev_room_id` uses. `Some("0")` is a real Kokoro voice, not
    /// a stand-in for "unset". Shape-checked by `parse_tts_voice` on write.
    #[serde(default)]
    pub tts_voice: Option<String>,
```

Update `Default` and every test in the file that names `tts_speaker_id`.

- [ ] **Step 3: Schema**

In `db/schema_settings.rs`, replace the `tts_speaker_id` comment and `ALTER` with:

```rust
    // Nullable: NULL means "no preference, use TTS_VOICE". A TEXT column
    // because a voice is whatever the configured provider calls one ("0" for
    // Kokoro, "alloy" for OpenAI, an id for ElevenLabs).
    let _ = crate::db_ddl!(pool, "ALTER TABLE user_settings ADD COLUMN tts_voice TEXT;");

    // The integer column this replaced. Dropped rather than migrated: the
    // project does not carry compatibility paths, and a saved Kokoro index
    // would be meaningless to any other provider. Best-effort like every
    // ALTER here: a fresh database never had it.
    let _ = crate::db_ddl!(pool, "ALTER TABLE user_settings DROP COLUMN tts_speaker_id;");
```

- [ ] **Step 4: Repository**

In `repository.rs`, rename `tts_speaker_id` to `tts_voice` in the `SELECT` column list, the `INSERT` column list, and the `ON CONFLICT ... tts_voice = excluded.tts_voice` clause. The bind becomes `&settings.tts_voice`. Its test uses `tts_voice: Some("alloy".to_string())` and asserts `created.tts_voice.as_deref() == Some("alloy")`.

- [ ] **Step 5: `!settings voice`**

In `handler.rs::set_voice`, keep the empty-value and `none`/`clear` branches. The parse branch becomes:

```rust
        match parse_tts_voice(value) {
            Ok(voice) => Some(voice),
            Err(reason) => {
                return format!(
                    "⚠️ {} (or <code>none</code> to clear).<br/>Example: <code>!settings voice alloy</code>",
                    html_escape(&reason)
                );
            }
        }
```

(Import `crate::utils::html::html_escape` if `handler.rs` does not already.) `updated.tts_voice = voice;`. The success reply is `format!("✅ Voice set to <b>{}</b>.", html_escape(&voice))`, and the `None` reply stays. Change the prompt text "A speaker index" to "A voice name or id". In `usage()`, change `!settings voice &lt;0-{}|none&gt;` to `!settings voice &lt;voice|none&gt;` and drop the `MAX_TTS_SPEAKER_ID` format argument. Replace the `MAX_TTS_SPEAKER_ID` import with `parse_tts_voice`.

In `modules/settings/mod.rs`, the `settings voice` `CommandHelp` becomes usage `"!settings voice <voice|none>"` and description `"Set the voice !say speaks in: a name or id your TTS provider knows (e.g. !settings voice 0)"`.

Rewrite the handler voice tests:

```rust
    #[tokio::test]
    async fn test_sets_voice_and_the_speaker_alias_works() {
        // As before, but "!settings voice alloy" then "!settings speaker 21m00Tcm4TlvDq8ikWAM",
        // asserting tts_voice == Some("alloy") then Some("21m00Tcm4TlvDq8ikWAM").
    }

    #[tokio::test]
    async fn test_sets_voice_to_zero_and_zero_survives() {
        // "!settings voice 0" -> tts_voice == Some("0").
    }

    #[tokio::test]
    async fn test_clearing_the_voice_resets_to_no_preference() {
        // "!settings voice alloy" then "!settings voice none" -> None.
    }

    #[tokio::test]
    async fn test_rejects_a_malformed_voice_without_changing_anything() {
        // Save "alloy", then each of "!settings voice a/b", "!settings voice ../x",
        // and a 65-character voice must reply with "⚠️" and leave Some("alloy").
    }

    #[tokio::test]
    async fn test_voice_broadcasts_on_success_and_not_on_rejection() {
        // As before with "!settings voice alloy" (broadcasts) and
        // "!settings voice a/b" (does not).
    }
```

These are the existing tests with the values listed in their comments substituted. Keep each test's existing structure and assertions, changing only the values and field name.

- [ ] **Step 6: API**

In `api/settings.rs`: the request field becomes

```rust
    /// The voice `!say` speaks in, as the configured TTS provider names it,
    /// or `null`/absent to clear back to the server default, the same
    /// convention `dev_room_id` uses.
    #[serde(default)]
    pub tts_voice: Option<String>,
```

Validation replaces the speaker-id `match`:

```rust
    let tts_voice = match payload.tts_voice.as_deref() {
        Some(raw) => Some(parse_tts_voice(raw).map_err(ApiError::bad_request)?),
        None => None,
    };
```

(Keep the existing comment above it, re-worded from "speaker id" to "voice". If `ApiError::bad_request` does not accept a `String` by value, use `|e| ApiError::bad_request(e)` as the neighbouring code does.) Pass `tts_voice` into `UserSettings`. Import `parse_tts_voice` in place of `MAX_TTS_SPEAKER_ID`. Rewrite the three API tests:
- `test_update_stores_a_valid_tts_voice`: saves `"0"`, `"alloy"` and a 64-character voice.
- `test_rejects_a_malformed_tts_voice`: `"a b"`, `"../x"` and 65 characters each return a 400.
- `test_omitting_tts_voice_clears_a_previously_saved_one`: the same flow as before.

Also rename the default-settings assertion (`settings.tts_speaker_id == None` becomes `settings.tts_voice == None`).

- [ ] **Step 7: `modules/tts`**

`resolve_voice(house_default: &str, saved: Option<&str>) -> String { saved.unwrap_or(house_default).to_string() }`. Call it with `settings.tts_voice.as_deref()`. Update its two tests to `resolve_voice("5", Some("77")) == "77"` and `resolve_voice("44", Some("0")) == "0"`.

- [ ] **Step 8: Regenerate the API artifacts**

Run: `cargo run --bin generate-openapi` then `npm --prefix frontend run generate-api`
Expected: `backend/openapi.json` and `frontend/src/types/api.ts` now carry `tts_voice` (string, nullable), and no `tts_speaker_id`.

- [ ] **Step 9: Frontend**

`frontend/src/core/settings.ts`: replace the `tts_speaker_id` field and its doc with

```ts
  /** The voice `!say` speaks in, as the configured TTS provider names it.
   *  `null`/absent means no preference: the server uses its own default.
   *  `"0"` is a real Kokoro voice, not a stand-in for "unset". */
  tts_voice?: string | null;
```

and `tts_speaker_id: null` becomes `tts_voice: null` in `DEFAULT_SETTINGS`.

`frontend/src/components/SettingsModal.tsx`:
- Replace the `MAX_TTS_SPEAKER_ID` constant and its doc with:

```tsx
/** Mirrors the server's `parse_tts_voice`, so a malformed voice is caught
 *  before a round trip. */
const TTS_VOICE_PATTERN = /^[A-Za-z0-9_-]{1,64}$/;
```

- `const [ttsVoice, setTtsVoice] = useState(saved.tts_voice ?? '');`, and in the reset effect `setTtsVoice(saved.tts_voice ?? '');`, with the effect's dependency `saved.tts_speaker_id` becoming `saved.tts_voice`.
- `dirty`: `(ttsVoice.trim() || null) !== (saved.tts_voice ?? null)`.
- At the top of `handleSave`, after `setErrorMsg(null);`:

```tsx
    const voice = ttsVoice.trim();
    if (voice && !TTS_VOICE_PATTERN.test(voice)) {
      setErrorMsg('A voice is 1–64 letters, digits, "_" or "-".');
      return;
    }
```

  and in the payload, `tts_voice: voice || null,`.
- Replace the voice `<input>` and hint with:

```tsx
          <input
            id="tts-voice"
            type="text"
            placeholder="server default"
            value={ttsVoice}
            onChange={(e) => setTtsVoice(e.target.value)}
            className="w-full px-3 py-2 rounded bg-neutral-50 dark:bg-neutral-900 border border-neutral-300 dark:border-neutral-800 text-sm text-neutral-900 dark:text-neutral-100 focus:outline-none focus:border-neutral-500 font-mono"
          />
          <p className="text-[11px] font-mono text-neutral-500">
            The voice <code>!say</code> speaks in, as your TTS provider names it: a Kokoro speaker
            id (0–102) on the bundled server, a name such as <code>alloy</code> on OpenAI, or a
            voice id on ElevenLabs. Leave empty to use the server default.
          </p>
```

  and the label's `htmlFor` becomes `"tts-voice"`.

- [ ] **Step 10: Verify**

Run: `grep -rn "tts_speaker_id\|MAX_TTS_SPEAKER_ID\|ttsSpeakerId" backend/src frontend/src backend/openapi.json`
Expected: no output.
Run: `cargo fmt --all`, then `cargo check --workspace --all-targets` (no warnings), then `cargo test -p matrix-bot` (all pass).
Run: `npm --prefix frontend run build`
Expected: success.
Run: `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
Expected: no diff after the regenerated file is staged.

- [ ] **Step 11: Commit**

```bash
git add backend frontend
git commit -m "feat!: store each user's TTS voice as a provider voice name instead of a Kokoro index"
```

---

### Task 8: Documentation

**Files:**
- Modify: `CLAUDE.md`, `README.md`, `docs/superpowers/specs/2026-09-22-inference-service-design.md`

- [ ] **Step 1: CLAUDE.md**

- In Repository Layout, change the `inference/` bullet's "(OCR and speech-to-text now; TTS later)" to "(OCR, speech-to-text and Kokoro text-to-speech)".
- In "Two crates, one deployment", add TTS alongside OCR and STT wherever the two are listed. Add one sentence: text to speech reaches `POST /v1/audio/speech` via `services/tts`, which speaks the OpenAI shape and, under `TTS_DIALECT=elevenlabs`, the ElevenLabs one.
- Rewrite the "Voice replies (`!say`, offline text-to-speech)" section. Rename it to "Voice replies (`!say`)" and have it state these facts, replacing the old engine and cmake paragraphs:
  1. `modules/tts` prepares the text (`text.rs` strips HTML and emoji, which no provider has a reading for), resolves the voice, and uploads the result. Synthesis goes through `services/tts` to the bundled Kokoro (`inference/src/kokoro.rs`) or any OpenAI-compatible or ElevenLabs provider, chosen by `TTS_BASE_URL` / `TTS_DIALECT`. An unset base URL makes `!say` answer that speech is unavailable, and nothing else changes.
  2. The reply is Ogg Opus, which is what makes a client draw a voice bubble. The bundled server adds `X-Duration-Ms` and `X-Waveform`. A provider that omits them still gets a bubble: `services::tts::ogg_opus_duration` reads the duration from the last page's granule minus the pre-skip, and the bubble has no bars. The granule arithmetic of the encoder (`inference/src/opus.rs`) is checked externally with `ffprobe` over the route's output.
  3. The per-user voice is `user_settings.tts_voice: Option<String>`, and `TTS_VOICE` is the house default. It is an `Option` because `"0"` is a real Kokoro voice. It is shape-checked only (`parse_tts_voice`), since the bot cannot know a provider's voices. The bundled server clamps an out-of-range Kokoro id (`kokoro::clamp_speaker_id`) and answers 400 to a non-integer voice.
  4. The Kokoro engine loads once into a `static OnceLock` (311 MB, seconds to load), unlike SenseVoice, which rebuilds per call. It is provisioned at inference startup from the sherpa-onnx `tts-models` release archive, unpacked into a staging directory and renamed into place. Its text-normalisation FSTs ship inside that archive.
  5. Building `matrix-bot-inference` needs `cmake` (the vendored libopus in `opusic-sys`); the bot needs neither `cmake` nor any sherpa shared library.
- Remove every remaining mention of `TTS_ENABLED`, `TTS_SPEAKER_ID`, `tts_speaker_id`, `engine::clamp_speaker_id` in `modules/tts`, and `tts_spike` (`grep -n "TTS_ENABLED\|TTS_SPEAKER_ID\|tts_speaker_id\|tts_spike\|modules/tts/engine\|modules/tts/opus" CLAUDE.md` must end empty). Also remove the Windows `onnxruntime.dll` note from any section describing the **bot**, and keep it for the inference crate.

- [ ] **Step 2: README.md**

- The features bullet (line ~26) becomes: "🔊 **Voice replies**: `!say <text>` speaks into the room as a native Matrix voice bubble (Ogg Opus). By default this is synthesized by the bundled inference server's Kokoro (English + Mandarin, 103 voices, Chinese text normalization), or by OpenAI or ElevenLabs with one env change. Every user picks their own voice."
- The commands table row: "Speaks the text into the room as a native Matrix voice message; voice chosen per user".
- The env block (line ~290): replace the three `TTS_*` lines with the `TTS_BASE_URL` / `TTS_API_KEY` / `TTS_MODEL` / `TTS_VOICE` / `TTS_DIALECT` lines, each with a short trailing comment, matching the style of the surrounding STT lines.
- The layout lines (~511–517): drop "TTS later"; the inference crate hosts OCR, STT and TTS.

- [ ] **Step 3: Spec amendments**

In `docs/superpowers/specs/2026-09-22-inference-service-design.md`:
- Under "Per-user voice", after the consequences list, add a paragraph in italics: *"Revised during Plan 3 execution: `TTS_ENABLED` is removed as well; an absent `TTS_BASE_URL` is the off switch, as for OCR and STT. There is no data migration from `tts_speaker_id`; the column is dropped (see CLAUDE.md's no-compatibility rule)."*
- Under "Phasing", item 3, append: *"Revised during Plan 3 execution: `text.rs` stays in `modules/tts`, because it prepares a Matrix message and helps every provider; `tts_spike.rs` is deleted, as its own header asked, not moved; Kokoro is provisioned from the sherpa-onnx release archive at startup, like the other models."*
- Under "Known unfixed risk", append: *"Checked on 2026-09-24 in a Linux build of the inference image: `OrtGetApiBase` is in the binary's `.symtab` but not its `.dynsym`, and `libsherpa-onnx-c-api.so` binds `OrtGetApiBase@VERS_1.28.2`, which its own `libonnxruntime.so` provides. The two runtimes do not interpose. Re-check after an `ort` or sherpa-onnx upgrade, or if the binary is ever linked with `-rdynamic`."*

- [ ] **Step 4: Verify and commit**

Run: `cargo fmt --all -- --check && cargo check --workspace --all-targets && cargo test -p matrix-bot && cargo test -p matrix-bot-inference && npm --prefix frontend run build`
Expected: all green, no warnings.

```bash
git add CLAUDE.md README.md docs/superpowers/specs/2026-09-22-inference-service-design.md
git commit -m "docs: document text-to-speech through the inference service"
```

---

## Acceptance (controller-run, after Task 8)

These need Docker and the real model, so they are not tasks for an implementer. The local Kokoro model under `data/tts_models/kokoro-multi-lang-v1_1` is copied into the inference volume, so the 365 MB download is not repeated.

1. `docker compose build inference`, then start the inference service alone with the local model mounted at `/app/data/tts_models/kokoro-multi-lang-v1_1`.
2. `curl /health` → `tts: true` (plus `stt` / `ocr` if their models are mounted).
3. `curl -X POST /v1/audio/speech` with `contracts/tts_speech_request.json` → 200, `content-type: audio/ogg`, both extension headers present. `ffprobe` on the body reports an Opus stream whose duration matches `x-duration-ms` within 40 ms.
4. In the same container, `POST /ocr` with a real image and then `/v1/audio/speech` again. Both answer, which proves all three engines coexist in one Linux process, not just in the symbol table.
5. `curl -X POST` with `"voice": "alloy"` → 400.

The OpenAI and ElevenLabs acceptance (a real voice bubble from a hosted provider) needs keys this environment does not have. The user runs it by setting `TTS_BASE_URL` / `TTS_API_KEY` / `TTS_MODEL` / `TTS_VOICE` (plus `TTS_DIALECT=elevenlabs`) and sending `!say`.
