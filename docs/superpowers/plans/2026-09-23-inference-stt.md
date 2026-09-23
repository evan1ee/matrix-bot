# Speech-to-Text Extraction — Implementation Plan (Plan 2 of 4)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move SenseVoice speech-to-text into `matrix-bot-inference` behind an OpenAI-compatible `POST /v1/audio/transcriptions`, have the bot reach it — or any OpenAI-compatible provider — through a new `services/stt` client, and drop `symphonia`, `opus-decoder`, `ort` and `ort-sys` from the bot crate.

**Architecture:** The inference crate gains the SenseVoice engine and audio decoder (copied first, deleted from the backend last, so every commit compiles), a multipart transcription route speaking OpenAI's wire format, and background model provisioning so a fresh deployment serves `/health` immediately. The backend gains `services/stt`, a `reqwest` multipart client configured by `STT_BASE_URL` / `STT_API_KEY` / `STT_MODEL`. `modules/voice` keeps its business logic — prompt, AI turn, Matrix reply — and only its transcription call becomes a network call. Both crates' wire types are pinned by shared JSON fixtures in `contracts/`.

**Tech Stack:** Rust, Axum 0.7 (`multipart`), reqwest 0.11 (`multipart`), sherpa-onnx 1.13.8 (shared), symphonia 0.5, tokio.

**Spec:** `docs/superpowers/specs/2026-09-22-inference-service-design.md`

**Branch:** `refactor/inference-stt`, stacked on `refactor/inference-service` (PR #15).

## Corrections to the spec's phasing

The spec's Phase 2 says STT drops `sherpa-onnx`, `symphonia` and `opus-decoder`. Checked against the code:

- **`sherpa-onnx` cannot leave the bot yet.** `backend/src/modules/tts/engine.rs` uses it for Kokoro. It leaves in Plan 3 with TTS. So the backend CI jobs keep their sherpa-onnx archive steps through this plan, and the inference CI job gains its own.
- **`ort` and `ort-sys` leave now.** `cargo tree -p matrix-bot -i ort` shows `matrix-bot` as their only dependent, nothing in the bot references `ort::`, and `sherpa-onnx` does not depend on `ort`. They existed to pin `paddle-ocr-rs`'s ONNX Runtime and have been dead since Plan 1 moved it. Their build script still downloads ONNX Runtime from `cdn.pyke.io` on every backend build.
- `ogg` stays in the bot: `modules/tts/opus.rs` uses it, and the spec keeps it for reading Ogg durations in Plan 3.

## Global Constraints

- **Every commit compiles, in both crates.** Run `cargo check --workspace` before every commit. Code moves as copy-then-delete across tasks, never as a move that leaves a crate broken.
- **A task that adds a native dependency to a crate also updates every build that compiles that crate** — its Dockerfile and its CI job — in the same task.
- **`services/` never imports from `modules/`.** Wire types stay separate from domain types; the `From` impl lives module-side.
- **No user-facing text contains upstream-derived detail.** No `err.to_string()` of a service error in a Matrix reply or an API error body. Full detail goes to `warn!`.
- **Error-to-status mappings are exhaustive matches** with no `_` catch-all over an enum this codebase owns.
- **Route tests go through `build_router`** with `tower::ServiceExt::oneshot`, not only handler calls.
- **Any test that loads a sherpa-onnx model is `#[ignore]`d.** On Windows, a mismatched `onnxruntime.dll` kills the whole test process with an access violation — see the doc comment on `test_synthesize_produces_audio_for_english_and_chinese` in `backend/src/modules/tts/engine.rs`.
- **Tests never touch the network.** A test that calls a provisioning function gives it a directory whose model files already exist.
- **f32 values in contract fixtures are exactly representable** (`0.5`, `0.75`, `1.5`); `0.98` as an `f32` does not round-trip through `serde_json::Value`.
- `backend/openapi.json` must not change. Verify with `cargo run --bin generate-openapi && git diff --ignore-all-space --exit-code backend/openapi.json`, then `git checkout -- backend/openapi.json` to discard the CRLF rewrite that command makes on Windows.
- `cargo check --workspace` warning-free; `cargo fmt --all -- --check` clean.
- No source file in `backend/src/` or `inference/src/` exceeds 500 lines of production code (above `#[cfg(test)]`).
- Never call `sqlx::query*` directly; use the `db_*!` macros.
- Workspace `default-members = ["backend"]` unchanged.
- Commit types `feat` / `fix` / `refactor` / `docs` / `test` / `chore` / `perf` / `ci`; every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Three tests in `backend/src/modules/ai/handler.rs` (`test_ai_error_suppressed_from_chat_and_sent_once`, `test_ai_quoted_context_prompt_routing`, `test_ai_mention_natural_language_routing`) race on process-global state. Pre-existing, tracked separately. Ignore their failures; any other failure is real.

## File Structure

**Created**

| Path | Responsibility |
|---|---|
| `contracts/ocr_scan.json` | The OCR wire contract, asserted by both crates. |
| `contracts/stt_transcription_verbose.json` | The STT `verbose_json` wire contract, asserted by both crates. |
| `inference/src/provision.rs` | Background download of every capability's models. |
| `inference/src/audio.rs` | Copied from `backend/src/modules/voice/audio.rs`: bytes → 16 kHz mono PCM. |
| `inference/src/sherpa.rs` | Copied from `backend/src/modules/voice/sherpa.rs`, plus its error/result types and `models_present`. |
| `inference/src/routes/transcriptions.rs` | `POST /v1/audio/transcriptions`, OpenAI's multipart shape. |
| `backend/src/services/test_support.rs` | `spawn_server`, shared by every `services/*` client test. |
| `backend/src/services/stt/mod.rs` | `SttClient` for any OpenAI-compatible transcription endpoint. |

**Modified**

| Path | Change |
|---|---|
| `inference/src/main.rs` | Spawns provisioning instead of awaiting it before bind. |
| `inference/src/config.rs` | Adds `voice_model_dir` (`VOICE_MODEL_DIR`). |
| `inference/src/routes/{mod,health,ocr}.rs` | Registers the route, reports real `stt` health, pins the OCR contract. |
| `inference/Cargo.toml` | Adds `sherpa-onnx`, `symphonia`, `ogg`, `opus-decoder`; axum `multipart`. |
| `Dockerfile.inference` | Ships the sherpa-onnx shared libraries; `VOICE_MODEL_DIR`. |
| `.github/workflows/ci.yml` | Inference job gains the sherpa-onnx archive steps. |
| `backend/src/services/{mod.rs,ocr/mod.rs}` | Declares `stt` and `test_support`; OCR tests use the shared helper and the contract. |
| `backend/src/modules/voice/{mod.rs,models.rs}` | Transcribes via `SttClient`; `VoiceConfig` loses `model_dir`; no upstream detail in replies. |
| `backend/src/api/voice.rs` | Transcribes via `state.stt`; fixed error bodies. |
| `backend/src/api/auth/types.rs`, `backend/src/bot/{attachments,handler}.rs`, `backend/src/config.rs` | Thread the client; `STT_*` config replaces `VOICE_MODEL_DIR`. |
| `backend/Cargo.toml` | Drops `symphonia`, `opus-decoder`, `ort`, `ort-sys`; reqwest `multipart`. |
| `docker-compose.yml`, `.env.example`, `CLAUDE.md`, `AGENTS.md`, `README.md`, `backend/src/modules/tts/engine.rs` (doc comment only) | Document it. |

**Deleted:** `backend/src/modules/voice/audio.rs`, `backend/src/modules/voice/sherpa.rs` (in Task 5, after the inference copies exist).

---

## Task 1: Provision models in the background

Plan 1 made provisioning blocking and pre-bind, so on a fresh volume the listener waits for every download — up to 120 s per OCR model against a 10 s health-check `start_period`. This plan adds a 239 MB SenseVoice download on top. Spawn it instead: the server answers `/health` immediately, and each route already gates on its own `models_present`, answering 503 until its files land. Downloads write to a `.tmp` file and rename, so a route never sees a half-written model.

**Files:**
- Create: `inference/src/provision.rs`
- Modify: `inference/src/main.rs`

**Interfaces:**
- Consumes: `crate::paddle::ensure_models(&Path) -> Result<(), OcrError>`, `crate::paddle::{DET_MODEL_NAME, CLS_MODEL_NAME, REC_MODEL_NAME}`, `crate::config::InferenceConfig { port, ocr_model_dir }`.
- Produces: `crate::provision::provision_models(config: Arc<InferenceConfig>)` — `async`, returns `()`, never fails. Task 3 extends its body; the signature stays.

- [ ] **Step 1: Write the failing test**

Create `inference/src/provision.rs`:

```rust
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::InferenceConfig;

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
```

Add `mod provision;` to `inference/src/main.rs` beside the other `mod` lines.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p matrix-bot-inference provision`
Expected: FAIL — `cannot find function 'provision_models' in this scope`.

- [ ] **Step 3: Implement `provision_models`**

Insert above the `#[cfg(test)]` block in `inference/src/provision.rs`:

```rust
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
```

- [ ] **Step 4: Spawn it from `main`**

In `inference/src/main.rs`, delete the whole `if let Err(err) = paddle::ensure_models(...)` block and the comment above it. After `let state = routes::AppState { ... };` and before `TcpListener::bind`, add:

```rust
    // Provisioning downloads models and can take minutes; run it beside the
    // server, not in front of it. See `provision::provision_models`.
    tokio::spawn(provision::provision_models(state.config.clone()));
```

Change `use tracing::{info, warn};` to `use tracing::info;` — `main.rs` no longer warns.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p matrix-bot-inference`
Expected: PASS, every existing test plus the new one.

Run: `cargo check --workspace --all-targets`
Expected: no warnings.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add inference/src/provision.rs inference/src/main.rs
git commit -m "fix: provision inference models in the background, not before bind

Awaiting the downloads before binding left a fresh container unhealthy for as
long as they took -- up to 120s per OCR model against a 10s start_period --
and the SenseVoice model this branch adds is 239 MB more. Spawn provisioning
beside the server instead. Each route already gates on its own models_present
and answers 503 until its files land, and downloads rename from a .tmp file,
so no request can see a partial model.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 2: Pin the OCR wire contract

`inference`'s `OcrScanBody` and the backend's `OcrScan` are separate types by design. Today their field names match only by convention: rename one and both crates compile, both suites pass, and OCR breaks at runtime. Pin both sides to one fixture file before the STT contract copies the pattern.

**Files:**
- Create: `contracts/ocr_scan.json`, `backend/src/services/test_support.rs`
- Modify: `inference/src/routes/ocr.rs` (tests), `backend/src/services/mod.rs`, `backend/src/services/ocr/mod.rs` (tests)

**Interfaces:**
- Consumes: `inference::routes::ocr::{OcrScanBody, OcrScanBlock}`, `backend::services::ocr::{OcrScan, OcrScanBlock}` — unchanged.
- Produces: `crate::services::test_support::spawn_server(app: axum::Router) -> String` (backend, `#[cfg(test)]`, `pub(crate)`), returning `http://127.0.0.1:<port>`. Task 4 uses it.

- [ ] **Step 1: Write the failing tests**

In `inference/src/routes/ocr.rs`, inside `mod tests`, add:

```rust
    const OCR_SCAN_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/ocr_scan.json"
    ));

    #[test]
    fn test_ocr_scan_body_matches_the_shared_wire_contract() {
        let body = OcrScanBody {
            text: "SHAKE SHACK\nTotal $16.50".to_string(),
            blocks: vec![
                OcrScanBlock {
                    text: "SHAKE SHACK".to_string(),
                    score: 0.5,
                },
                OcrScanBlock {
                    text: "Total $16.50".to_string(),
                    score: 0.75,
                },
            ],
        };

        let expected: serde_json::Value =
            serde_json::from_str(OCR_SCAN_CONTRACT).expect("the contract is valid JSON");
        assert_eq!(serde_json::to_value(&body).unwrap(), expected);
    }
```

In `backend/src/services/ocr/mod.rs`, inside `mod tests`, add:

```rust
    const OCR_SCAN_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/ocr_scan.json"
    ));

    #[test]
    fn test_the_shared_wire_contract_deserialises_into_ocr_scan() {
        let scan: OcrScan =
            serde_json::from_str(OCR_SCAN_CONTRACT).expect("the contract should parse");

        assert_eq!(
            scan,
            OcrScan {
                text: "SHAKE SHACK\nTotal $16.50".to_string(),
                blocks: vec![
                    OcrScanBlock {
                        text: "SHAKE SHACK".to_string(),
                        score: 0.5,
                    },
                    OcrScanBlock {
                        text: "Total $16.50".to_string(),
                        score: 0.75,
                    },
                ],
            }
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p matrix-bot-inference ocr_scan_body_matches`
Expected: FAIL — `couldn't read ...contracts/ocr_scan.json`.

- [ ] **Step 3: Create the fixture**

Create `contracts/ocr_scan.json`:

```json
{
  "text": "SHAKE SHACK\nTotal $16.50",
  "blocks": [
    { "text": "SHAKE SHACK", "score": 0.5 },
    { "text": "Total $16.50", "score": 0.75 }
  ]
}
```

- [ ] **Step 4: Extract the shared test server**

Create `backend/src/services/test_support.rs`:

```rust
//! Helpers shared by the `services/*` client tests.

/// Starts a throwaway server on an ephemeral port and returns its base URL.
/// Uses axum and tokio, both already backend dependencies, rather than adding
/// a mock-HTTP crate.
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

In `backend/src/services/mod.rs`, below `pub mod ocr;`, add:

```rust
#[cfg(test)]
mod test_support;
```

In `backend/src/services/ocr/mod.rs` tests, delete the local `async fn spawn_server` and add `use crate::services::test_support::spawn_server;` to the test module's imports.

- [ ] **Step 5: Run both suites**

Run: `cargo test -p matrix-bot-inference` and `cargo test -p matrix-bot services::ocr`
Expected: PASS, including both contract tests.

- [ ] **Step 6: Prove the contract bites**

Temporarily rename `text` to `content` on `OcrScanBody` in `inference/src/routes/ocr.rs` (field and every use). Run `cargo test -p matrix-bot-inference ocr_scan_body_matches` — it must FAIL. Revert with `git checkout -- inference/src/routes/ocr.rs`, then re-run to confirm PASS. Record both outcomes in the report.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add contracts/ocr_scan.json inference/src/routes/ocr.rs backend/src/services
git commit -m "test: pin the OCR wire contract to a fixture both crates assert

OcrScanBody and OcrScan are separate types on purpose, so nothing stopped a
field rename on either side from compiling, passing both suites, and breaking
OCR at runtime. contracts/ocr_scan.json is now serialised against on the
server and deserialised on the client. Also extracts the client tests' ad hoc
server into services/test_support.rs for the STT client to reuse.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 3: Serve SenseVoice at `POST /v1/audio/transcriptions`

**Files:**
- Create: `inference/src/audio.rs`, `inference/src/sherpa.rs` (copies), `inference/src/routes/transcriptions.rs`, `contracts/stt_transcription_verbose.json`
- Modify: `inference/Cargo.toml`, `inference/src/main.rs`, `inference/src/config.rs`, `inference/src/provision.rs`, `inference/src/routes/mod.rs`, `inference/src/routes/health.rs`, `inference/src/routes/ocr.rs` (test literals only), `Dockerfile.inference`, `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: `provision_models` (Task 1); the `contracts/` pattern (Task 2).
- Produces:
  - `InferenceConfig { port: u16, ocr_model_dir: PathBuf, voice_model_dir: PathBuf }` — `voice_model_dir` from `VOICE_MODEL_DIR`, default `data/voice_models`.
  - `crate::sherpa::{VoiceError, VoiceTranscriptionResult, models_present, ensure_models_downloaded, run_transcription, run_transcription_sync, SENSEVOICE_MODEL_FILE, SENSEVOICE_TOKENS_FILE}`.
  - Route `POST /v1/audio/transcriptions`, multipart fields `file` (required), `language` (optional; absent → `auto`), `response_format` (optional; absent or `json` → `{"text"}`, `verbose_json` → `{"task","language","duration","text"}`, anything else → 400), `model` and all other fields accepted and ignored. Statuses: 400 missing file, bad format or undecodable audio; 503 models not installed; 500 inference failure.
  - `crate::audio::test_audio::silent_wav_16k_mono(seconds: f32) -> Vec<u8>` (`#[cfg(test)]`, `pub(crate)`).

- [ ] **Step 1: Add the dependencies**

In `inference/Cargo.toml`, change `axum = { version = "0.7" }` to `axum = { version = "0.7", features = ["multipart"] }`, and add to `[dependencies]` — the same versions and features the backend uses:

```toml
sherpa-onnx = { version = "1.13.8", default-features = false, features = ["shared"] }
symphonia = { version = "0.5", default-features = false, features = ["ogg", "vorbis", "wav", "mp3", "aac", "isomp4", "pcm", "flac"] }
ogg = "0.9"
opus-decoder = "0.1"
```

- [ ] **Step 2: Copy the engine and decoder**

Copy, do not move — the backend still compiles against its originals until Task 5:

```bash
cp backend/src/modules/voice/audio.rs inference/src/audio.rs
cp backend/src/modules/voice/sherpa.rs inference/src/sherpa.rs
```

In `inference/src/audio.rs`, change `use super::models::VoiceError;` to `use crate::sherpa::VoiceError;`.

In `inference/src/sherpa.rs`, change `use super::audio::TARGET_SAMPLE_RATE;` to `use crate::audio::TARGET_SAMPLE_RATE;`, delete `use super::models::{VoiceError, VoiceTranscriptionResult};`, and add these types below the imports. `Disabled` is dropped (whether a feature is enabled is the bot's concern, and an unconstructed variant warns in a bin crate), and so are `is_empty` and the `Display` impl (unused here):

```rust
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
```

Add `mod audio;` and `mod sherpa;` to `inference/src/main.rs`.

- [ ] **Step 3: Write the failing engine tests**

In `inference/src/audio.rs`, add at the end of the file:

```rust
#[cfg(test)]
pub(crate) mod test_audio {
    /// A mono 16-bit PCM WAV of silence at 16 kHz: enough for the decoder to
    /// accept and for SenseVoice to run over, with no audio fixture on disk.
    pub(crate) fn silent_wav_16k_mono(seconds: f32) -> Vec<u8> {
        let sample_rate: u32 = 16_000;
        let data_len = (sample_rate as f32 * seconds) as u32 * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
        wav.extend_from_slice(&2u16.to_le_bytes()); // block align
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.resize(44 + data_len as usize, 0);
        wav
    }
}
```

Inside the existing `mod tests` in `inference/src/audio.rs`, add:

```rust
    #[test]
    fn test_a_silent_wav_decodes_to_a_second_of_zeroes() {
        let wav = super::test_audio::silent_wav_16k_mono(1.0);

        let samples = decode_audio_to_16k_mono(&wav).expect("a PCM WAV should decode");

        assert_eq!(samples.len(), 16_000);
        assert!(samples.iter().all(|s| s.abs() < 1e-6));
    }
```

Inside the existing `mod tests` in `inference/src/sherpa.rs`, add:

```rust
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
```

- [ ] **Step 4: Run them to verify they fail**

Run: `cargo test -p matrix-bot-inference models_present`
Expected: FAIL — `cannot find function 'models_present'` in `sherpa`.

- [ ] **Step 5: Implement `models_present`**

In `inference/src/sherpa.rs`, below the two `SENSEVOICE_*` constants:

```rust
/// Whether both files SenseVoice needs are on disk. The route gates on this
/// and answers 503 until provisioning has finished.
pub fn models_present(model_dir: &Path) -> bool {
    model_dir.join(SENSEVOICE_MODEL_FILE).is_file()
        && model_dir.join(SENSEVOICE_TOKENS_FILE).is_file()
}
```

Run: `cargo test -p matrix-bot-inference audio sherpa`
Expected: PASS (the `#[ignore]`d test reports as ignored).

- [ ] **Step 6: Add `voice_model_dir` to the config and provisioning**

In `inference/src/config.rs`, add the field `pub voice_model_dir: PathBuf,`, default `PathBuf::from("data/voice_models")`, and in `from_env`:

```rust
        let voice_model_dir = std::env::var("VOICE_MODEL_DIR")
            .map(PathBuf::from)
            .unwrap_or(defaults.voice_model_dir);
```

In `inference/src/provision.rs`, append to `provision_models`:

```rust
    match crate::sherpa::ensure_models_downloaded(&config.voice_model_dir).await {
        Ok(()) => info!("Speech-to-text models ready in {:?}", config.voice_model_dir),
        Err(err) => warn!(
            "Speech-to-text model provisioning failed; transcription reports unavailable \
             until this is resolved: {}",
            err
        ),
    }
```

Update the provisioning test: `provisioned_dir` also writes `crate::sherpa::SENSEVOICE_MODEL_FILE` and `crate::sherpa::SENSEVOICE_TOKENS_FILE`, and the config literal gains `voice_model_dir: dir.clone()`. **Without both files the test downloads 239 MB** — it must not.

Every other `InferenceConfig { .. }` literal in the crate's tests (`routes/health.rs`, `routes/ocr.rs`) gains `voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models")`. Use an explicit nonexistent path. Do not use `..InferenceConfig::default()`: the default `data/ocr_models` resolves through `paddle::resolve_model_dir`'s parent fallback to the real models on a dev machine and flips the health assertions.

- [ ] **Step 7: Report real `stt` health**

In `inference/src/routes/health.rs`, replace `stt: false,` with `stt: crate::sherpa::models_present(&state.config.voice_model_dir),`, and change the handler's doc comment to say `tts` alone is hardcoded false until the TTS plan. The existing health tests keep asserting `stt: false`, now because the directory is absent.

- [ ] **Step 8: Write the contract fixture and the failing route tests**

Create `contracts/stt_transcription_verbose.json`:

```json
{
  "task": "transcribe",
  "language": "zh",
  "duration": 1.5,
  "text": "今天天气真好。"
}
```

Create `inference/src/routes/transcriptions.rs` with the wire types and the tests. The handler follows in Step 10.

```rust
use axum::{
    extract::{Multipart, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use tracing::warn;

use super::AppState;
use crate::sherpa::VoiceError;

/// The `response_format` values this server supports. OpenAI defines more
/// (`text`, `srt`, `vtt`); the bot asks only for `verbose_json`, and `json` is
/// OpenAI's default when the field is absent.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ResponseFormat {
    Json,
    VerboseJson,
}

/// The `json` response: text only.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TranscriptionText {
    pub text: String,
}

/// The `verbose_json` response, as OpenAI defines it minus `segments`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TranscriptionVerbose {
    pub task: &'static str,
    pub language: Option<String>,
    pub duration: f32,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(untagged)]
pub enum TranscriptionBody {
    Json(TranscriptionText),
    Verbose(TranscriptionVerbose),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use axum::body::Body;
    use axum::http::Request;
    use std::sync::Arc;
    use tower::ServiceExt;

    const STT_VERBOSE_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/stt_transcription_verbose.json"
    ));
    const BOUNDARY: &str = "matrix-bot-test-boundary";

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
                voice_model_dir: std::path::PathBuf::from("/nonexistent/voice/models"),
            }),
        }
    }

    /// A multipart/form-data body; each part is (name, filename, bytes).
    fn multipart_body(parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, bytes) in parts {
            body.extend_from_slice(format!("--{}\r\n", BOUNDARY).as_bytes());
            let disposition = match filename {
                Some(f) => format!(
                    "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\r\n",
                    name, f
                ),
                None => format!("Content-Disposition: form-data; name=\"{}\"\r\n\r\n", name),
            };
            body.extend_from_slice(disposition.as_bytes());
            body.extend_from_slice(bytes);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{}--\r\n", BOUNDARY).as_bytes());
        body
    }

    async fn post_transcription(parts: &[(&str, Option<&str>, &[u8])]) -> StatusCode {
        let request = Request::builder()
            .method("POST")
            .uri("/v1/audio/transcriptions")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={}", BOUNDARY),
            )
            .body(Body::from(multipart_body(parts)))
            .unwrap();
        crate::routes::build_router(state_with_missing_models())
            .oneshot(request)
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn test_undecodable_audio_is_a_400_even_without_models() {
        let status = post_transcription(&[("file", Some("a.ogg"), b"not audio")]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_decodable_audio_without_models_is_a_503() {
        let wav = crate::audio::test_audio::silent_wav_16k_mono(1.0);
        let status =
            post_transcription(&[("file", Some("a.wav"), &wav), ("model", None, b"whisper-1")])
                .await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn test_a_request_without_a_file_is_a_400() {
        let status = post_transcription(&[("model", None, b"whisper-1")]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_an_unsupported_response_format_is_a_400() {
        let wav = crate::audio::test_audio::silent_wav_16k_mono(1.0);
        let status = post_transcription(&[
            ("file", Some("a.wav"), &wav),
            ("response_format", None, b"srt"),
        ])
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_response_format_defaults_to_json_as_openai_does() {
        assert_eq!(ResponseFormat::parse(None), Ok(ResponseFormat::Json));
        assert_eq!(ResponseFormat::parse(Some("json")), Ok(ResponseFormat::Json));
        assert_eq!(
            ResponseFormat::parse(Some("verbose_json")),
            Ok(ResponseFormat::VerboseJson)
        );
        assert!(ResponseFormat::parse(Some("srt")).is_err());
    }

    #[test]
    fn test_status_for_every_voice_error() {
        let s = |e: VoiceError| status_for(&e);
        assert_eq!(s(VoiceError::AudioDecode("x".into())), StatusCode::BAD_REQUEST);
        assert_eq!(s(VoiceError::MissingModel("x".into())), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(s(VoiceError::ModelDownload("x".into())), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(s(VoiceError::Inference("x".into())), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(s(VoiceError::Io("x".into())), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_verbose_body_matches_the_shared_wire_contract() {
        let body = TranscriptionBody::Verbose(TranscriptionVerbose {
            task: "transcribe",
            language: Some("zh".to_string()),
            duration: 1.5,
            text: "今天天气真好。".to_string(),
        });

        let expected: serde_json::Value =
            serde_json::from_str(STT_VERBOSE_CONTRACT).expect("the contract is valid JSON");
        assert_eq!(serde_json::to_value(&body).unwrap(), expected);
    }

    #[test]
    fn test_json_body_is_text_only() {
        let body = TranscriptionBody::Json(TranscriptionText {
            text: "hi".to_string(),
        });
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            serde_json::json!({ "text": "hi" })
        );
    }
}
```

Register it: in `inference/src/routes/mod.rs` add `pub mod transcriptions;` and `.route("/v1/audio/transcriptions", post(transcriptions::transcribe))`.

- [ ] **Step 9: Run them to verify they fail**

Run: `cargo test -p matrix-bot-inference transcriptions`
Expected: FAIL — `transcribe`, `status_for` and `ResponseFormat::parse` are not defined.

- [ ] **Step 10: Implement the route**

Insert into `inference/src/routes/transcriptions.rs`, above `#[cfg(test)]`:

```rust
impl ResponseFormat {
    fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None | Some("json") => Ok(Self::Json),
            Some("verbose_json") => Ok(Self::VerboseJson),
            Some(other) => Err(format!(
                "Unsupported response_format '{}'; expected json or verbose_json",
                other
            )),
        }
    }
}

struct TranscriptionRequest {
    file: Vec<u8>,
    language: Option<String>,
    format: ResponseFormat,
}

/// Reads the multipart form. `model` and every other OpenAI field are accepted
/// and ignored: this server has exactly one model.
async fn read_request(
    mut multipart: Multipart,
) -> Result<TranscriptionRequest, (StatusCode, String)> {
    let bad = |message: String| (StatusCode::BAD_REQUEST, message);
    let mut file = None;
    let mut language = None;
    let mut format = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad(format!("Malformed multipart body: {}", e)))?
    {
        let name = field.name().map(str::to_owned);
        match name.as_deref() {
            Some("file") => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| bad(format!("Failed to read 'file': {}", e)))?;
                file = Some(bytes.to_vec());
            }
            Some("language") => {
                language = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad(format!("Failed to read 'language': {}", e)))?,
                );
            }
            Some("response_format") => {
                format = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| bad(format!("Failed to read 'response_format': {}", e)))?,
                );
            }
            _ => {}
        }
    }

    let file = file
        .filter(|f| !f.is_empty())
        .ok_or_else(|| bad("Missing or empty 'file' field".to_string()))?;
    let format = ResponseFormat::parse(format.as_deref()).map_err(bad)?;
    let language = language
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty());

    Ok(TranscriptionRequest {
        file,
        language,
        format,
    })
}

/// Maps an engine error to a status. Exhaustive on purpose: a new
/// `VoiceError` variant must be a compile error here, not a silent 500.
fn status_for(err: &VoiceError) -> StatusCode {
    match err {
        VoiceError::AudioDecode(_) => StatusCode::BAD_REQUEST,
        VoiceError::MissingModel(_) | VoiceError::ModelDownload(_) => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        VoiceError::Inference(_) | VoiceError::Io(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `POST /v1/audio/transcriptions` — OpenAI's transcription endpoint, served
/// by SenseVoice. The audio is decoded before the model gate, so a malformed
/// upload is a 400 whether or not models are installed: the caller's error is
/// reported before the server's state.
pub async fn transcribe(
    State(state): State<AppState>,
    multipart: Multipart,
) -> Result<Json<TranscriptionBody>, (StatusCode, String)> {
    let request = read_request(multipart).await?;

    let samples = crate::audio::decode_audio_to_16k_mono(&request.file)
        .map_err(|e| (status_for(&e), e.to_string()))?;

    if !crate::sherpa::models_present(&state.config.voice_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Speech-to-text models are not installed on this inference server".to_string(),
        ));
    }

    let language = request.language.unwrap_or_else(|| "auto".to_string());
    let result =
        crate::sherpa::run_transcription(samples, state.config.voice_model_dir.clone(), language)
            .await
            .map_err(|e| {
                warn!("SenseVoice transcription failed: {:?}", e);
                (status_for(&e), e.to_string())
            })?;

    let body = match request.format {
        ResponseFormat::Json => TranscriptionBody::Json(TranscriptionText { text: result.text }),
        ResponseFormat::VerboseJson => TranscriptionBody::Verbose(TranscriptionVerbose {
            task: "transcribe",
            language: result.language,
            duration: result.duration_secs,
            text: result.text,
        }),
    };
    Ok(Json(body))
}
```

The error body here is `e.to_string()`, the same convention as `POST /ocr`. The inference server is internal, and the bot never relays these bodies to a person: it logs them and shows fixed text (Task 5).

- [ ] **Step 11: Run the inference suite**

Run: `cargo test -p matrix-bot-inference`
Expected: PASS; one test ignored.

Run the ignored real-model test by hand. On this machine the models are in `data/voice_models/` and `target/debug/deps/onnxruntime.dll` is already present:

```bash
cargo test -p matrix-bot-inference real_sensevoice -- --ignored
```

Expected: PASS. Record in the report whether it ran or skipped, and its time — a sub-10 ms pass means it skipped.

Run `cargo test -p matrix-bot-inference real_image` too. The OCR test must still pass now that sherpa-onnx's shared ONNX Runtime is linked into the same binary as `ort`'s static one.

- [ ] **Step 12: Ship the shared libraries in the image**

In `Dockerfile.inference`, directly after the `COPY --from=build ... /usr/local/bin/matrix-bot-inference` line and before `RUN mkdir -p /app/data`:

```dockerfile
# sherpa-onnx is linked as a shared library, so its C API and the ONNX Runtime
# it was built against ship beside the binary -- the same two files the bot
# image copies. `ort` (PaddleOCR) links its own ONNX Runtime statically and
# needs nothing here. ldconfig runs as root, so this precedes `USER app`.
COPY --from=build /src/target/release/libsherpa-onnx-c-api.so /usr/local/lib/
COPY --from=build /src/target/release/libonnxruntime.so       /usr/local/lib/
RUN ldconfig
```

In the `ENV` block, add `VOICE_MODEL_DIR=/app/data/voice_models` beside `OCR_MODEL_DIR`.

- [ ] **Step 13: Give the inference CI job the sherpa-onnx archive steps**

In `.github/workflows/ci.yml`, in the `inference` job, insert these four steps between `Swatinem/rust-cache@v2` and `Format check`. They are the backend job's steps, and the backend job keeps its copy, because `modules/tts` still links sherpa-onnx:

```yaml
      # sherpa-onnx arrives in this crate with speech-to-text. Same handling as
      # the backend jobs: rust-cache prunes the prebuilt .so files, so the
      # build script is forced to re-run, and the archive it fetches is cached
      # so the download is a cache-miss-only path with retries.
      - name: Resolve the sherpa-onnx version
        id: sherpa
        run: |
          set -euo pipefail
          version="$(grep -A1 'name = "sherpa-onnx-sys"' Cargo.lock | grep '^version' | cut -d'"' -f2)"
          test -n "$version"
          echo "version=$version" >> "$GITHUB_OUTPUT"
          echo "archive=sherpa-onnx-v$version-linux-x64-shared-lib.tar.bz2" >> "$GITHUB_OUTPUT"
          echo "SHERPA_ONNX_ARCHIVE_DIR=$RUNNER_TEMP/sherpa-archive" >> "$GITHUB_ENV"

      - name: Cache the sherpa-onnx prebuilt archive
        uses: actions/cache@v4
        with:
          path: ${{ runner.temp }}/sherpa-archive
          key: sherpa-onnx-${{ steps.sherpa.outputs.version }}-linux-x64-shared

      - name: Fetch the sherpa-onnx archive if the cache missed
        run: |
          set -euo pipefail
          dir="$RUNNER_TEMP/sherpa-archive"
          name="${{ steps.sherpa.outputs.archive }}"
          mkdir -p "$dir"
          if [ -f "$dir/$name" ]; then
            echo "archive already cached: $name"
            exit 0
          fi
          curl --proto '=https' --tlsv1.2 --location --fail --show-error --silent \
               --retry 5 --retry-all-errors --retry-delay 5 \
               --output "$dir/$name" \
               "https://github.com/k2-fsa/sherpa-onnx/releases/download/v${{ steps.sherpa.outputs.version }}/$name"

      - name: Clean sherpa-onnx prebuilt cache
        run: |
          cargo clean -p sherpa-onnx-sys -p sherpa-onnx || true
          rm -rf target/sherpa-onnx-prebuilt
```

The job's existing "Install Linux build dependencies" comment says nothing in this crate builds with cmake. That stays true: `sherpa-onnx-sys` downloads a prebuilt archive, and `opusic-sys` is not in this crate.

- [ ] **Step 14: Verify, then commit**

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test -p matrix-bot-inference
git add Cargo.lock inference/ contracts/stt_transcription_verbose.json Dockerfile.inference .github/workflows/ci.yml
git commit -m "feat: serve SenseVoice at an OpenAI-compatible /v1/audio/transcriptions

Copies the SenseVoice engine and audio decoder into the inference crate --
copied, not moved, so the backend keeps compiling until it is cut over -- and
serves them in OpenAI's multipart shape, so the bot can point the same client
at this server or at any hosted provider. Undecodable audio is a 400 whether or
not models are installed; missing models are a 503.

Ships sherpa-onnx's shared libraries in the inference image and gives its CI
job the same cached archive steps the backend jobs use. The verbose_json shape
is pinned to contracts/stt_transcription_verbose.json.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 4: The `services/stt` client

**Files:**
- Create: `backend/src/services/stt/mod.rs`
- Modify: `backend/src/services/mod.rs`, `backend/Cargo.toml`

**Interfaces:**
- Consumes: `crate::services::test_support::spawn_server` (Task 2); `contracts/stt_transcription_verbose.json` (Task 3).
- Produces:
  - `SttClient::new(base_url: Option<String>, api_key: Option<String>, model: String) -> SttClient` — blank strings count as absent.
  - `SttClient::transcribe(&self, audio: Vec<u8>, mime_type: Option<&str>, language: Option<&str>) -> Result<SttTranscription, SttServiceError>`
  - `SttTranscription { text: String, language: Option<String>, duration: Option<f32> }`
  - `SttServiceError::{NotConfigured, Unreachable(String), Upstream { status: u16, message: String }}`
  - `file_extension_for(mime_type: Option<&str>) -> &'static str`

- [ ] **Step 1: Enable reqwest multipart**

In `backend/Cargo.toml`, add `"multipart"` to the `reqwest` features list.

- [ ] **Step 2: Write the failing tests**

Add `pub mod stt;` to `backend/src/services/mod.rs`. Create `backend/src/services/stt/mod.rs` with the types and the tests; the implementation follows in Step 4.

```rust
use serde::Deserialize;
use std::fmt;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// Bounds a hung connection; SenseVoice on a CPU and hosted providers both
/// answer a voice note well inside this.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// OpenAI's `verbose_json` transcription, reduced to what the bot reads.
/// `language` and `duration` are optional because a provider answering plain
/// `json` sends text only.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct SttTranscription {
    pub text: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub duration: Option<f32>,
}

#[derive(Debug, Error)]
pub enum SttServiceError {
    #[error("Speech-to-text service is not configured")]
    NotConfigured,
    #[error("Speech-to-text service is unreachable: {0}")]
    Unreachable(String),
    #[error("Speech-to-text service returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::test_support::spawn_server;
    use axum::{
        extract::Multipart,
        http::{HeaderMap, StatusCode},
        routing::post,
        Router,
    };
    use std::sync::{Arc, Mutex};

    const STT_VERBOSE_CONTRACT: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../contracts/stt_transcription_verbose.json"
    ));

    /// What the fake server saw: the Authorization header and every
    /// multipart field as (name, filename, bytes).
    #[derive(Debug, Default)]
    struct Captured {
        authorization: Option<String>,
        fields: Vec<(String, Option<String>, Vec<u8>)>,
    }

    impl Captured {
        fn field(&self, name: &str) -> Option<&(String, Option<String>, Vec<u8>)> {
            self.fields.iter().find(|(n, _, _)| n == name)
        }
    }

    /// A fake OpenAI-compatible server that records the request and answers
    /// with the shared contract. Returns the base URL including `/v1`.
    async fn capturing_server(captured: Arc<Mutex<Captured>>) -> String {
        let app = Router::new().route(
            "/v1/audio/transcriptions",
            post(move |headers: HeaderMap, mut multipart: Multipart| {
                let captured = captured.clone();
                async move {
                    let mut record = Captured {
                        authorization: headers
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned),
                        fields: Vec::new(),
                    };
                    while let Some(field) = multipart.next_field().await.unwrap() {
                        let name = field.name().unwrap_or_default().to_owned();
                        let filename = field.file_name().map(str::to_owned);
                        let bytes = field.bytes().await.unwrap().to_vec();
                        record.fields.push((name, filename, bytes));
                    }
                    *captured.lock().unwrap() = record;
                    (
                        StatusCode::OK,
                        [("content-type", "application/json")],
                        STT_VERBOSE_CONTRACT,
                    )
                }
            }),
        );
        format!("{}/v1", spawn_server(app).await)
    }

    fn contract_transcription() -> SttTranscription {
        SttTranscription {
            text: "今天天气真好。".to_string(),
            language: Some("zh".to_string()),
            duration: Some(1.5),
        }
    }

    #[tokio::test]
    async fn test_an_unconfigured_client_reports_not_configured() {
        let client = SttClient::new(None, None, "sensevoice-small".to_string());
        assert!(matches!(
            client.transcribe(vec![1, 2, 3], None, None).await,
            Err(SttServiceError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn test_sends_the_audio_filename_model_and_verbose_format() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = capturing_server(captured.clone()).await;
        let client = SttClient::new(Some(base), None, "sensevoice-small".to_string());

        let result = client
            .transcribe(vec![7, 8, 9], Some("audio/ogg; codecs=opus"), None)
            .await
            .expect("transcription should succeed");

        assert_eq!(result, contract_transcription());
        let seen = captured.lock().unwrap();
        let (_, filename, bytes) = seen.field("file").expect("a file part was sent");
        assert_eq!(bytes.as_slice(), &[7u8, 8, 9]);
        assert_eq!(filename.as_deref(), Some("audio.ogg"));
        assert_eq!(seen.field("model").unwrap().2, b"sensevoice-small");
        assert_eq!(seen.field("response_format").unwrap().2, b"verbose_json");
        assert!(seen.field("language").is_none(), "no hint means no language field");
        assert!(seen.authorization.is_none(), "no key means no Authorization header");
    }

    #[tokio::test]
    async fn test_sends_the_language_hint_and_bearer_token_when_given() {
        let captured = Arc::new(Mutex::new(Captured::default()));
        let base = capturing_server(captured.clone()).await;
        let client = SttClient::new(
            Some(base),
            Some("sk-test".to_string()),
            "whisper-1".to_string(),
        );

        client
            .transcribe(vec![1], Some("audio/mpeg"), Some("zh"))
            .await
            .expect("transcription should succeed");

        let seen = captured.lock().unwrap();
        assert_eq!(seen.authorization.as_deref(), Some("Bearer sk-test"));
        assert_eq!(seen.field("language").unwrap().2, b"zh");
        assert_eq!(seen.field("file").unwrap().1.as_deref(), Some("audio.mp3"));
    }

    #[test]
    fn test_the_shared_wire_contract_deserialises_into_stt_transcription() {
        let parsed: SttTranscription =
            serde_json::from_str(STT_VERBOSE_CONTRACT).expect("the contract should parse");
        assert_eq!(parsed, contract_transcription());
    }

    #[test]
    fn test_a_text_only_response_still_parses() {
        let parsed: SttTranscription = serde_json::from_str(r#"{"text":"hi"}"#).unwrap();
        assert_eq!(parsed.text, "hi");
        assert_eq!(parsed.language, None);
        assert_eq!(parsed.duration, None);
    }

    #[tokio::test]
    async fn test_an_upstream_error_carries_its_status_and_message() {
        let app = Router::new().route(
            "/v1/audio/transcriptions",
            post(|| async { (StatusCode::SERVICE_UNAVAILABLE, "models are not installed") }),
        );
        let base = format!("{}/v1", spawn_server(app).await);
        let client = SttClient::new(Some(base), None, "sensevoice-small".to_string());

        match client.transcribe(vec![1], None, None).await {
            Err(SttServiceError::Upstream { status, message }) => {
                assert_eq!(status, 503);
                assert!(message.contains("not installed"));
            }
            other => panic!("expected an upstream error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_an_unreachable_service_is_not_a_panic() {
        let client = SttClient::new(
            Some("http://127.0.0.1:1/v1".to_string()),
            None,
            "sensevoice-small".to_string(),
        );
        assert!(matches!(
            client.transcribe(vec![1], None, None).await,
            Err(SttServiceError::Unreachable(_))
        ));
    }

    #[test]
    fn test_debug_output_never_contains_the_api_key() {
        let client = SttClient::new(
            Some("https://api.openai.com/v1".to_string()),
            Some("sk-very-secret".to_string()),
            "whisper-1".to_string(),
        );
        let debug = format!("{:?}", client);
        assert!(!debug.contains("sk-very-secret"), "leaked: {}", debug);
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn test_file_extension_for_known_and_unknown_mime_types() {
        assert_eq!(file_extension_for(Some("audio/ogg; codecs=opus")), "ogg");
        assert_eq!(file_extension_for(Some("audio/mpeg")), "mp3");
        assert_eq!(file_extension_for(Some("audio/x-m4a")), "m4a");
        assert_eq!(file_extension_for(Some("audio/mp4")), "m4a");
        assert_eq!(file_extension_for(Some("AUDIO/WAV")), "wav");
        assert_eq!(file_extension_for(Some("audio/webm")), "webm");
        assert_eq!(file_extension_for(Some("audio/flac")), "flac");
        assert_eq!(file_extension_for(Some("application/x-unknown")), "ogg");
        assert_eq!(file_extension_for(None), "ogg");
    }
}
```

- [ ] **Step 3: Run them to verify they fail**

Run: `cargo test -p matrix-bot services::stt`
Expected: FAIL — `SttClient` and `file_extension_for` are not defined.

- [ ] **Step 4: Implement the client**

Insert above `#[cfg(test)]` in `backend/src/services/stt/mod.rs`:

```rust
/// Client for any OpenAI-compatible `POST {base}/audio/transcriptions`: the
/// bundled inference server, OpenAI, Groq, speaches. `base_url` includes the
/// version prefix (`.../v1`), the way OpenAI SDKs take it.
///
/// An absent `base_url` is a normal state, not an error condition: it means
/// transcription is not configured, and every call reports `NotConfigured` so
/// the caller can degrade that one feature.
#[derive(Clone)]
pub struct SttClient {
    base_url: Option<String>,
    api_key: Option<String>,
    model: String,
    http: reqwest::Client,
}

/// Written by hand so the bearer token never reaches a `{:?}` log line.
impl fmt::Debug for SttClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SttClient")
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model", &self.model)
            .finish()
    }
}

impl SttClient {
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

        Self {
            base_url,
            api_key,
            model,
            http,
        }
    }

    /// Sends the audio untouched and returns its transcription.
    ///
    /// The part's filename carries an extension derived from `mime_type`,
    /// because OpenAI identifies the audio format from it; the bundled server
    /// probes the bytes and ignores it. `language` is sent only when given.
    pub async fn transcribe(
        &self,
        audio: Vec<u8>,
        mime_type: Option<&str>,
        language: Option<&str>,
    ) -> Result<SttTranscription, SttServiceError> {
        let base = self
            .base_url
            .as_ref()
            .ok_or(SttServiceError::NotConfigured)?;

        let file = reqwest::multipart::Part::bytes(audio)
            .file_name(format!("audio.{}", file_extension_for(mime_type)));
        let mut form = reqwest::multipart::Form::new()
            .part("file", file)
            .text("model", self.model.clone())
            .text("response_format", "verbose_json");
        if let Some(language) = language {
            form = form.text("language", language.to_string());
        }

        let mut request = self
            .http
            .post(format!("{}/audio/transcriptions", base))
            .multipart(form);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }

        let response = request.send().await.map_err(|e| {
            warn!("Speech-to-text request failed: {}", e);
            SttServiceError::Unreachable(e.to_string())
        })?;

        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(SttServiceError::Upstream {
                status: status.as_u16(),
                message,
            });
        }

        response
            .json::<SttTranscription>()
            .await
            .map_err(|e| SttServiceError::Upstream {
                status: status.as_u16(),
                message: format!("Malformed transcription response: {}", e),
            })
    }
}

/// The file extension OpenAI needs to recognise an audio format. Matrix voice
/// notes are Ogg Opus, which is also the fallback for anything unrecognised.
pub fn file_extension_for(mime_type: Option<&str>) -> &'static str {
    let essence = mime_type
        .and_then(|m| m.split(';').next())
        .map(|m| m.trim().to_ascii_lowercase());

    match essence.as_deref() {
        Some("audio/mpeg") | Some("audio/mp3") => "mp3",
        Some("audio/mp4") | Some("audio/x-m4a") | Some("audio/m4a") => "m4a",
        Some("audio/wav") | Some("audio/x-wav") | Some("audio/wave") => "wav",
        Some("audio/webm") => "webm",
        Some("audio/flac") | Some("audio/x-flac") => "flac",
        _ => "ogg",
    }
}
```

- [ ] **Step 5: Run the tests**

Run: `cargo test -p matrix-bot services::stt`
Expected: PASS, nine tests.

Run: `cargo check --workspace --all-targets`
Expected: no warnings. `SttClient` has no production caller until Task 5, but it is `pub` in a lib crate, so it does not warn.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add backend/Cargo.toml Cargo.lock backend/src/services
git commit -m "feat: add the services/stt client for OpenAI-compatible transcription

One client for the bundled inference server and any hosted provider: STT base
URL, optional bearer token, model name. The upload's filename carries an
extension derived from the MIME type, because OpenAI identifies the format by
it. Debug is written by hand so the token cannot reach a log line. The request
shape is asserted by a fake server that records every multipart field, and the
response shape by the shared contract fixture.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 5: Cut the bot over and drop the native dependencies

**Files:**
- Modify: `backend/src/modules/voice/models.rs`, `backend/src/modules/voice/mod.rs`, `backend/src/api/voice.rs`, `backend/src/api/auth/types.rs`, `backend/src/bot/attachments.rs`, `backend/src/bot/handler.rs`, `backend/src/config.rs`, `backend/Cargo.toml`
- Delete: `backend/src/modules/voice/audio.rs`, `backend/src/modules/voice/sherpa.rs`

**Interfaces:**
- Consumes: `services::stt::{SttClient, SttTranscription, SttServiceError}` (Task 4).
- Produces:
  - `VoiceConfig { enabled: bool, language: String }` with an explicit `impl Default` (`true`, `"auto"`) and `fn language_hint(&self) -> Option<&str>`.
  - `impl From<SttTranscription> for VoiceTranscriptionResult` (module-side).
  - `process_audio_event(db, ai, broadcaster, room, owner_id, audio_bytes: &[u8], mime_type: Option<&str>, config: &VoiceConfig, stt: &SttClient, stage_reporter)`.
  - `Config { stt_base_url: Option<String>, stt_api_key: Option<String>, stt_model: String, .. }`; `voice_model_dir` removed.
  - `ApiState { stt: SttClient, .. }`.

Locate code by content, not line number. Catch every `Config { .. }` and `ApiState` literal, including in `#[cfg(test)]` modules: `grep -rn "voice_model_dir\|VoiceConfig {" backend/src` lists them.

- [ ] **Step 1: Write the failing tests**

In `backend/src/modules/voice/models.rs`, add at the end:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_hint_never_sends_auto_or_blank() {
        let hint = |language: &str| {
            VoiceConfig {
                enabled: true,
                language: language.to_string(),
            }
            .language_hint()
            .map(str::to_owned)
        };
        assert_eq!(hint("auto"), None);
        assert_eq!(hint("AUTO"), None);
        assert_eq!(hint("  "), None);
        assert_eq!(hint(""), None);
        assert_eq!(hint("zh"), Some("zh".to_string()));
        assert_eq!(hint(" en "), Some("en".to_string()));
    }

    #[test]
    fn test_voice_config_defaults_to_enabled_with_auto_detection() {
        let config = VoiceConfig::default();
        assert!(config.enabled);
        assert_eq!(config.language, "auto");
    }

    #[test]
    fn test_a_transcription_without_a_duration_maps_to_zero() {
        let result: VoiceTranscriptionResult = crate::services::stt::SttTranscription {
            text: "hi".to_string(),
            language: None,
            duration: None,
        }
        .into();
        assert_eq!(result.text, "hi");
        assert_eq!(result.duration_secs, 0.0);
    }
}
```

In `backend/src/modules/voice/mod.rs` `mod tests`, add:

```rust
    #[test]
    fn test_no_voice_error_reply_carries_upstream_detail() {
        use crate::services::stt::SttServiceError;
        let leaky = "http://inference:8090/v1/audio/transcriptions symphonia-detail";
        let cases = [
            (SttServiceError::NotConfigured, "语音处理不可用"),
            (SttServiceError::Unreachable(leaky.to_string()), "语音处理不可用"),
            (
                SttServiceError::Upstream { status: 400, message: leaky.to_string() },
                "语音无法识别",
            ),
            (
                SttServiceError::Upstream { status: 503, message: leaky.to_string() },
                "语音处理不可用",
            ),
            (
                SttServiceError::Upstream { status: 401, message: leaky.to_string() },
                "语音处理不可用",
            ),
        ];
        for (err, headline) in cases {
            let html = voice_error_message(&err);
            assert!(html.contains(headline), "{:?} -> {}", err, html);
            assert!(!html.contains("inference:8090"), "leaked a URL: {}", html);
            assert!(!html.contains("symphonia-detail"), "leaked detail: {}", html);
        }
    }
```

In `backend/src/api/voice.rs` `mod tests`, add (`ApiError` exposes `status: StatusCode` and `message: String`, as the `ocr_error_response` tests in `api/ocr.rs` rely on):

```rust
    use crate::services::stt::SttServiceError;
    use axum::http::StatusCode;

    #[test]
    fn test_voice_error_response_status_for_every_arm() {
        let status = |err: SttServiceError| voice_error_response(err).status;
        let upstream = |code: u16| SttServiceError::Upstream {
            status: code,
            message: "detail".to_string(),
        };

        assert_eq!(status(SttServiceError::NotConfigured), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(status(upstream(400)), StatusCode::BAD_REQUEST);
        assert_eq!(status(upstream(503)), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(status(upstream(401)), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            status(SttServiceError::Unreachable("refused".to_string())),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn test_voice_error_response_never_relays_upstream_text() {
        let decode = voice_error_response(SttServiceError::Upstream {
            status: 400,
            message: "symphonia-detail: unsupported codec".to_string(),
        });
        assert!(!decode.message.contains("symphonia-detail"));

        let unreachable = voice_error_response(SttServiceError::Unreachable(
            "error sending request for url (http://inference:8090/v1/audio/transcriptions)"
                .to_string(),
        ));
        assert!(!unreachable.message.contains("inference:8090"));
    }
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p matrix-bot voice`
Expected: FAIL — `language_hint`, `voice_error_message` and `voice_error_response` are not defined.

- [ ] **Step 3: Rework `VoiceConfig` and the result mapping**

Replace the top of `backend/src/modules/voice/models.rs` (the imports, `VoiceConfig` and its `Default`) with the version below. Delete `VoiceError` and its `From<std::io::Error>` impl: their producers are the decoder and engine, which leave in Step 7, and nothing outside `modules/voice` names them. Keep `VoiceTranscriptionResult`, its `is_empty` and its `Display` impl.

```rust
use std::fmt;

/// Voice transcription settings the bot owns. Where the model runs, and which
/// provider serves it, is `services::stt`'s concern.
#[derive(Debug, Clone)]
pub struct VoiceConfig {
    pub enabled: bool,
    pub language: String,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            language: "auto".to_string(),
        }
    }
}

impl VoiceConfig {
    /// The language to ask the transcription service for, or `None` to let it
    /// detect. `auto` is SenseVoice's own word for detection, and
    /// OpenAI-compatible providers reject it, so it is never sent.
    pub fn language_hint(&self) -> Option<&str> {
        let language = self.language.trim();
        if language.is_empty() || language.eq_ignore_ascii_case("auto") {
            None
        } else {
            Some(language)
        }
    }
}

impl From<crate::services::stt::SttTranscription> for VoiceTranscriptionResult {
    fn from(t: crate::services::stt::SttTranscription) -> Self {
        Self {
            text: t.text,
            language: t.language,
            duration_secs: t.duration.unwrap_or(0.0),
        }
    }
}
```

- [ ] **Step 4: Rewrite the module call site**

In `backend/src/modules/voice/mod.rs`:

- Remove `pub mod audio;` and `pub mod sherpa;`, and change the re-export to `pub use models::{VoiceConfig, VoiceTranscriptionResult};`.
- Change `process_audio_event`'s parameters so that, after `audio_bytes: &[u8]`, they read `mime_type: Option<&str>, config: &VoiceConfig, stt: &crate::services::stt::SttClient, stage_reporter: ...`.
- Replace its doc-comment steps 1 and 2 with one step: "Sends the audio to the configured speech-to-text service (`services::stt`)."
- Replace the whole decode block and the whole `sherpa::run_transcription` block with:

```rust
    let transcription: VoiceTranscriptionResult = match stt
        .transcribe(audio_bytes.to_vec(), mime_type, config.language_hint())
        .await
    {
        Ok(t) => t.into(),
        Err(err) => {
            warn!("Voice transcription failed for owner {}: {:?}", owner_id, err);
            return voice_error_message(&err);
        }
    };
```

- Add, above `format_voice_turn_response`:

```rust
/// The reply for a failed transcription. Never includes the error's own text:
/// that carries the service's response body, and on a connection failure the
/// request URL, neither of which belongs in a chat room. Full detail is in the
/// `warn!` at the call site.
fn voice_error_message(err: &crate::services::stt::SttServiceError) -> String {
    use crate::services::stt::SttServiceError;

    match err {
        SttServiceError::NotConfigured => "⚠️ <b>语音处理不可用</b><br/>\
             • <b>原因:</b> 本部署未配置语音识别服务<br/>\
             • <b>建议:</b> 请直接输入文字，或联系管理员启用语音识别。"
            .to_string(),
        SttServiceError::Upstream { status: 400, .. } => "⚠️ <b>语音无法识别</b><br/>\
             • <b>原因:</b> 音频格式无法解析<br/>\
             • <b>建议:</b> 请尝试重新录制并发送语音，或直接输入文字。"
            .to_string(),
        SttServiceError::Unreachable(_) | SttServiceError::Upstream { .. } => {
            "⚠️ <b>语音处理不可用</b><br/>\
             • <b>原因:</b> 语音识别服务当前不可用<br/>\
             • <b>建议:</b> 请稍后重试，或直接输入文字。"
                .to_string()
        }
    }
}
```

The AI-turn failure branch further down still formats the AI provider's error. That is a different service and is out of scope here; leave it.

- [ ] **Step 5: Rewrite the API call site**

In `backend/src/api/voice.rs`:

- Replace `use crate::modules::voice::{audio, sherpa};` with `use crate::modules::voice::VoiceTranscriptionResult;`.
- In the multipart loop, capture the part's content type before its bytes consume it:

```rust
    let mut upload: Option<(Vec<u8>, Option<String>)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::bad_request(format!("Failed to read multipart payload: {}", e)))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "file" || name == "audio" || name.is_empty() {
            let content_type = field.content_type().map(str::to_owned);
            let bytes = field
                .bytes()
                .await
                .map_err(|e| ApiError::bad_request(format!("Failed to read audio bytes: {}", e)))?;
            upload = Some((bytes.to_vec(), content_type));
            break;
        }
    }

    let (bytes, content_type) = upload
        .filter(|(bytes, _)| !bytes.is_empty())
        .ok_or_else(|| ApiError::bad_request("No audio file provided in multipart request"))?;
```

- Replace everything from the decode call through the `spawn_blocking` transcription with:

```rust
    info!("Transcribing voice audio ({} bytes)", bytes.len());

    let result: VoiceTranscriptionResult = state
        .stt
        .transcribe(bytes, content_type.as_deref(), state.voice_config.language_hint())
        .await
        .map_err(voice_error_response)?
        .into();
```

- Add, below the handler:

```rust
/// Maps a transcription failure to a response. Every body is a fixed string:
/// the error's own text carries the service's response body and, on a
/// connection failure, the request URL. Detail goes to the log instead.
fn voice_error_response(err: crate::services::stt::SttServiceError) -> ApiError {
    use crate::services::stt::SttServiceError;

    match err {
        SttServiceError::NotConfigured => ApiError::service_unavailable(
            "Voice transcription service is not configured on this server",
        ),
        SttServiceError::Upstream { status, message } => {
            warn!("Voice transcription failed upstream ({}): {}", status, message);
            match status {
                400 => ApiError::bad_request("The uploaded audio could not be decoded"),
                503 => ApiError::service_unavailable(
                    "The voice transcription service has no models installed",
                ),
                _ => ApiError::internal("Voice transcription failed"),
            }
        }
        SttServiceError::Unreachable(detail) => {
            warn!("Voice transcription service unreachable: {}", detail);
            ApiError::internal("Voice transcription failed")
        }
    }
}
```

`VoiceTranscribeResponse` and the `#[utoipa::path]` annotation are unchanged, so `openapi.json` must not move.

- [ ] **Step 6: Thread the client through config, state and the attachment path**

`backend/src/config.rs` — delete the `voice_model_dir` field, its `VOICE_MODEL_DIR` parse and its entries in every literal. Add below `voice_language`:

```rust
    /// Base URL of an OpenAI-compatible speech-to-text API, including its
    /// version prefix (`http://inference:8090/v1`, `https://api.openai.com/v1`).
    /// Absent means voice transcription is unavailable in this deployment.
    pub stt_base_url: Option<String>,
    /// Bearer token for a hosted STT provider. The bundled server needs none.
    pub stt_api_key: Option<String>,
    /// Model name sent with every transcription request. The bundled server
    /// has one model and ignores it.
    pub stt_model: String,
```

Parse them beside the other voice settings:

```rust
        let stt_base_url = env::var("STT_BASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let stt_api_key = env::var("STT_API_KEY")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        let stt_model = env::var("STT_MODEL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_STT_MODEL.to_string());
```

with `const DEFAULT_STT_MODEL: &str = "sensevoice-small";` near the top of the file. In test literals use `stt_base_url: None, stt_api_key: None, stt_model: "sensevoice-small".to_string()`.

`backend/src/api/auth/types.rs` — add `pub stt: crate::services::stt::SttClient,` after `pub ocr: ...`. In `new()`, add `stt: crate::services::stt::SttClient::new(None, None, "sensevoice-small".to_string()),`. In `from_config`, add:

```rust
            stt: crate::services::stt::SttClient::new(
                config.stt_base_url.clone(),
                config.stt_api_key.clone(),
                config.stt_model.clone(),
            ),
```

and drop `model_dir` from the `VoiceConfig { .. }` literal. Update `production_config()` to the new `Config` fields.

`backend/src/bot/attachments.rs` — the `Audio` variant gains `stt: &'a crate::services::stt::SttClient`. The enum stays borrowing, as `ocr` does. `from_msgtype` and `handle_attachment` each gain an `stt` parameter after `voice_config`. The `run_module` arm becomes:

```rust
            Self::Audio {
                content,
                voice_config,
                stt,
            } => {
                crate::modules::voice::process_audio_event(
                    db,
                    ai,
                    broadcaster,
                    room,
                    owner_id,
                    media_bytes,
                    content.info.as_ref().and_then(|info| info.mimetype.as_deref()),
                    voice_config,
                    stt,
                    Some(reporter),
                )
                .await
            }
```

`backend/src/bot/handler.rs` — build the client once beside `ocr_client`, clone it into the handler closure the way `ocr_client` is, drop `model_dir` from the `VoiceConfig` literal, and pass `&stt_client` to `handle_attachment`:

```rust
    let stt_client = crate::services::stt::SttClient::new(
        config.stt_base_url.clone(),
        config.stt_api_key.clone(),
        config.stt_model.clone(),
    );
```

- [ ] **Step 7: Delete the originals and drop the dependencies**

```bash
git rm backend/src/modules/voice/audio.rs backend/src/modules/voice/sherpa.rs
```

In `backend/Cargo.toml`, delete these four lines. Keep `sherpa-onnx` (`modules/tts/engine.rs`), `ogg` and `opus` (`modules/tts/opus.rs`):

```toml
ort = "=2.0.0-rc.10"
ort-sys = "=2.0.0-rc.10"
opus-decoder = "0.1"
symphonia = { version = "0.5", default-features = false, features = ["ogg", "vorbis", "wav", "mp3", "aac", "isomp4", "pcm", "flac"] }
```

- [ ] **Step 8: Verify everything**

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test -p matrix-bot
cargo test -p matrix-bot-inference
cargo tree -p matrix-bot --depth 1 | grep -E " (ort|ort-sys|symphonia|opus-decoder) v" || echo "all four removed"
cargo tree -p matrix-bot --depth 1 | grep -E " (sherpa-onnx|ogg|opus) v"
cargo run --bin generate-openapi && git diff --ignore-all-space --exit-code backend/openapi.json
git checkout -- backend/openapi.json
```

Expected: warning-free; both suites pass (apart from the known flaky three); `all four removed`; the second `cargo tree` lists `sherpa-onnx`, `ogg` and `opus`; `openapi.json` unchanged.

- [ ] **Step 9: Commit**

```bash
git add -A backend Cargo.lock
git commit -m "refactor: transcribe voice over HTTP and drop four native crates

modules/voice keeps its prompt, AI turn and reply, and only its transcription
call becomes a network call through services/stt -- which reaches the bundled
inference server or any OpenAI-compatible provider. No reply or API error body
carries the service's text or URL any more; detail goes to the log.

Drops symphonia and opus-decoder, which only decoded audio for SenseVoice, and
ort and ort-sys, which have had no dependent since PaddleOCR left in the
previous branch but still downloaded ONNX Runtime on every build. sherpa-onnx
stays until TTS moves: modules/tts uses it for Kokoro.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Task 6: Deploy and document it

**Files:**
- Modify: `docker-compose.yml`, `.env.example`, `CLAUDE.md`, `AGENTS.md`, `README.md`, `backend/src/modules/tts/engine.rs` (doc comment only)

**Interfaces:**
- Consumes: everything above.
- Produces: a compose deployment where voice notes transcribe out of the box, with any OpenAI-compatible provider one `.env` change away.

- [ ] **Step 1: Compose**

In `docker-compose.yml`, add to the `matrix-bot` service's `environment`, below `OCR_BASE_URL`. Compose has no `env_file:`, so a variable reaches the container only if it is listed here:

```yaml
      - STT_BASE_URL=${STT_BASE_URL:-http://inference:8090/v1}
      - STT_API_KEY=${STT_API_KEY:-}
      - STT_MODEL=${STT_MODEL:-sensevoice-small}
```

Add to the `inference` service's `environment`, below `OCR_MODEL_DIR`:

```yaml
      - VOICE_MODEL_DIR=/app/data/voice_models
```

- [ ] **Step 2: `.env.example`**

Replace the whole `# --- Voice Transcription ...` block, up to the TTS header, with:

```bash
# --- Voice Transcription (OpenAI-compatible speech-to-text) -------------------
# Enable/disable voice transcription for m.audio in Matrix chat (default: true)
VOICE_ENABLED=true
# Language hint sent with each request: auto, zh, en, ja, ko, yue
# (default: auto). "auto" is never sent -- the provider detects the language.
VOICE_LANGUAGE=auto

# Base URL of an OpenAI-compatible speech-to-text API, INCLUDING its /v1
# prefix. Read by the bot. Leave commented out to run without transcription:
# voice notes then answer with a message saying so, and nothing else changes.
#
# Under `docker compose up` this is unnecessary: docker-compose.yml defaults
# it to the bundled inference server (http://inference:8090/v1). Set it to use
# a hosted provider instead, for example:
#   STT_BASE_URL=https://api.openai.com/v1       STT_MODEL=whisper-1
#   STT_BASE_URL=https://api.groq.com/openai/v1  STT_MODEL=whisper-large-v3
# STT_BASE_URL=http://inference:8090/v1
# Bearer token for a hosted provider. The bundled server needs none.
# STT_API_KEY=
# Model name sent with each request (default: sensevoice-small, which the
# bundled server ignores -- it has exactly one model).
# STT_MODEL=sensevoice-small

# Read by the inference container, not the bot. Directory the SenseVoice model
# is downloaded into at inference-server startup.
VOICE_MODEL_DIR=/app/data/voice_models
```

- [ ] **Step 3: `CLAUDE.md`**

In "Two crates, one deployment", replace the paragraph beginning "OCR reaches the inference server over HTTP via `services/ocr`." with:

```markdown
OCR reaches the inference server over HTTP via `services/ocr`, and speech to
text via `services/stt`, which speaks OpenAI's `POST /v1/audio/transcriptions`
— so `STT_BASE_URL` can point at the bundled server, OpenAI, Groq or anything
else with that shape, and `STT_API_KEY` is sent as a bearer token. An unset
base URL, or an unreachable server, degrades that one feature: the bot still
boots, and `docker run` with a volume still yields a working bot without it.

The inference server provisions models in the background (`provision.rs`),
after it binds, so `/health` answers at once and each route reports 503 until
its own files land. Every wire contract between the two crates is a JSON
fixture in `contracts/` that a test on each side asserts, because the two
sides' types are separate by design and nothing else would notice a renamed
field.
```

In "Voice replies", change "deliberately unlike `modules/voice/sherpa.rs`, which rebuilds its recogniser on every call" to "deliberately unlike `inference/src/sherpa.rs`, which rebuilds its SenseVoice recogniser on every call".

- [ ] **Step 4: `engine.rs` doc comment**

In `backend/src/modules/tts/engine.rs`, in the doc comment on `test_synthesize_produces_audio_for_english_and_chinese`, replace "This is why `modules/voice` has no test that loads its model either." with "This is why `inference/src/sherpa.rs`'s SenseVoice test is `#[ignore]`d too." Doc-comment change only.

- [ ] **Step 5: `AGENTS.md` and `README.md`**

In `AGENTS.md`, add to the conventions list:

```markdown
- **Every HTTP wire contract between the crates lives in `contracts/`.** A JSON
  fixture that the server side serialises against and the client side
  deserialises from, so a renamed field fails a test instead of production.
```

In `README.md`'s project-tree section, add `services/stt/` beside `services/ocr/` and a top-level `contracts/` entry, and in the deployment section note that voice transcription can use a hosted provider by setting `STT_BASE_URL`, `STT_API_KEY` and `STT_MODEL`.

- [ ] **Step 6: Verify the deployment surfaces**

```bash
docker compose config > /dev/null && echo "compose OK"
cargo fmt --all -- --check
```

Reproduce both Dockerfiles' dependency layers outside Docker, exactly as their `RUN` steps build them: copy `Cargo.toml`, `Cargo.lock` and both member manifests into a scratch directory, create the same dummy sources, and run `cargo build --release --bin matrix-bot` (bot image) and `cargo build --release -p matrix-bot-inference` (inference image) far enough to prove the workspace loads — the first `Compiling` line, not a manifest error. Report both outcomes.

- [ ] **Step 7: Commit**

```bash
git add docker-compose.yml .env.example CLAUDE.md AGENTS.md README.md backend/src/modules/tts/engine.rs
git commit -m "docs: deploy and document speech-to-text over the inference server

Compose points the bot at the bundled server by default and passes STT_API_KEY
and STT_MODEL through, so a hosted provider is one .env change away.
STT_BASE_URL ships commented out in .env.example, so a bare docker run
degrades cleanly instead of failing to resolve a compose hostname.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 8: Hand-off acceptance (performed by the user, not the implementer)**

The spec's Phase 2 acceptance includes transcribing through a third-party provider. That needs a real API key, which the implementer must not source. List it in the report as outstanding:

1. `docker compose up --build`, then `docker compose exec matrix-bot curl -f http://inference:8090/health` — `stt` becomes `true` once the download finishes.
2. Send a voice note in Matrix; confirm the transcription reply.
3. Set `STT_BASE_URL=https://api.groq.com/openai/v1`, `STT_API_KEY=<key>`, `STT_MODEL=whisper-large-v3` (or OpenAI with `whisper-1`), restart the bot, and send another voice note.
4. `docker compose stop inference` with the default config, send a voice note, and confirm the "语音处理不可用" reply while `!todo list` still works.
