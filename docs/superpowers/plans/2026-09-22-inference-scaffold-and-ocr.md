# Inference Crate Scaffold + OCR Extraction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stand up the `matrix-bot-inference` workspace member and move PaddleOCR into it, so the bot calls OCR over HTTP and `image` + `paddle-ocr-rs` leave `backend/Cargo.toml`.

**Architecture:** A second Cargo workspace member serves an Axum API on port 8090. The backend gains a `services/` layer whose first member, `services/ocr`, is a `reqwest` client for that API. `modules/ocr` keeps every line of its business logic — the AI turn, the prompt, the Matrix reply formatting — and only its one call to `paddle::run_ocr` becomes a network call. An unset `OCR_BASE_URL`, or an unreachable service, degrades OCR alone and leaves the rest of the bot untouched.

**Tech Stack:** Rust, Cargo workspace, Axum 0.7, reqwest (rustls), tokio, serde, thiserror, paddle-ocr-rs, Docker Compose.

**Spec:** `docs/superpowers/specs/2026-09-22-inference-service-design.md` (committed as `6c1b94f`)

**Branch:** `refactor/inference-service`

## Global Constraints

- No source file in `backend/src/` exceeds **500 lines** of production code (lines above the `#[cfg(test)]` module); target under 300. From `AGENTS.md`.
- `cargo check` must be **warning-free**.
- Workspace `default-members` stays `["backend"]`, and `default-run = "matrix-bot"` stays in `backend/Cargo.toml`. Bare `cargo run` from the repo root must still start the bot.
- All REST error responses use `ApiError` from `backend/src/error.rs`.
- Never call `sqlx::query*` directly; use the `db_*!` macros in `backend/src/db/executor.rs`. (No database work in this plan, but it applies to any incidental change.)
- Commit messages use conventional types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `ci`.
- Every commit ends with the trailer `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`.
- After changing any `#[utoipa::path]` annotation or `ToSchema` struct, regenerate both artifacts: `cargo run --bin generate-openapi` then `npm --prefix frontend run generate-api`.
- No new third-party crate is added to `backend/` for testing. HTTP clients are tested against a throwaway Axum server on an ephemeral port, using the `axum` and `tokio` already present.

## File Structure

**Created**

| Path | Responsibility |
|---|---|
| `inference/Cargo.toml` | Crate manifest. Owns `image`, `paddle-ocr-rs`, `ort`, `ort-sys`. |
| `inference/src/main.rs` | Binds the listener, builds the router, reads config. |
| `inference/src/config.rs` | `InferenceConfig` from env: port, model dirs. |
| `inference/src/routes/mod.rs` | Router assembly and shared `AppState`. |
| `inference/src/routes/health.rs` | `GET /health` → per-capability model presence. |
| `inference/src/routes/ocr.rs` | `POST /ocr` → raw image bytes in, JSON out. |
| `inference/src/paddle.rs` | Moved verbatim from `backend/src/modules/ocr/paddle.rs`, plus `models_present`. |
| `backend/src/services/mod.rs` | Declares the services layer. |
| `backend/src/services/ocr/mod.rs` | `OcrClient` — the HTTP client and its wire DTOs. |
| `Dockerfile.inference` | Build + runtime image for the inference crate. |

**Modified**

| Path | Change |
|---|---|
| `Cargo.toml` | `members = ["backend", "inference"]`; `default-members` unchanged. |
| `backend/Cargo.toml` | Remove `image`, `paddle-ocr-rs`. `ort`/`ort-sys` stay until Task 4 of the TTS plan. |
| `backend/src/lib.rs` | `pub mod services;` |
| `backend/src/config.rs` | Add `ocr_base_url`; keep `ocr_enabled`, drop `ocr_model_dir`. |
| `backend/src/modules/ocr/mod.rs` | `paddle::run_ocr` → `OcrClient::scan`; `pub mod paddle` removed. |
| `backend/src/modules/ocr/models.rs` | `OcrConfig` loses `model_dir`, gains `base_url`. |
| `backend/src/api/ocr.rs` | Same call-site swap. `extract_receipt_fields` untouched. |
| `backend/src/api/auth/types.rs` | `ApiState` carries an `OcrClient`. |
| `backend/src/bot/attachments.rs` | `Image { model_dir }` → `Image { ocr: OcrClient }`. |
| `.github/workflows/ci.yml` | Backend jobs scoped to `-p matrix-bot`; new `inference` job. |
| `docker-compose.yml` | New `inference` service on an internal network. |
| `.env.example`, `README.md`, `CLAUDE.md`, `AGENTS.md` | Document the two-crate layout. |

**Deleted**

- `backend/src/modules/ocr/paddle.rs` (moved, not rewritten)

---

## Task 1: Workspace scaffold — the inference crate boots and answers `/health`

**Files:**
- Create: `inference/Cargo.toml`, `inference/src/main.rs`, `inference/src/config.rs`, `inference/src/routes/mod.rs`, `inference/src/routes/health.rs`
- Modify: `Cargo.toml`
- Test: `inference/src/routes/health.rs` (in-file `#[cfg(test)]`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `inference::config::InferenceConfig { port: u16, ocr_model_dir: PathBuf }`, built by `InferenceConfig::from_env() -> Self`
  - `inference::routes::AppState { config: Arc<InferenceConfig> }`
  - `inference::routes::build_router(state: AppState) -> axum::Router`
  - `inference::routes::health::HealthResponse { stt: bool, tts: bool, ocr: bool }`

- [ ] **Step 1: Add the workspace member**

Edit `Cargo.toml` at the repo root. Change only the `members` list; `default-members` must stay as it is, or bare `cargo run` stops resolving to the bot.

```toml
[workspace]
members = [
    "backend",
    "inference",
]
default-members = [
    "backend",
]
resolver = "2"
```

- [ ] **Step 2: Create the crate manifest**

Create `inference/Cargo.toml`. Versions match `backend/Cargo.toml` so the workspace resolves one copy of each.

```toml
[package]
name = "matrix-bot-inference"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = { version = "0.7", features = ["multipart"] }
tokio = { version = "1", features = ["full"] }
tower-http = { version = "0.5", features = ["trace", "limit"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
anyhow = "1"
```

- [ ] **Step 3: Write the config module**

Create `inference/src/config.rs`.

```rust
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
```

- [ ] **Step 4: Write the failing health test**

Create `inference/src/routes/health.rs` with the test only — the handler comes in Step 6, so this will not compile yet. That is the RED state.

```rust
use axum::{extract::State, Json};
use serde::{Deserialize, Serialize};

use super::AppState;

/// Per-capability model presence. A capability reports `false` when its model
/// files are absent, which is how an operator tells "not installed" from
/// "installed but failing".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResponse {
    pub stt: bool,
    pub tts: bool,
    pub ocr: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_health_reports_ocr_absent_when_models_are_missing() {
        let config = InferenceConfig {
            port: 0,
            ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
        };
        let state = AppState {
            config: Arc::new(config),
        };

        let Json(body) = health(State(state)).await;

        assert_eq!(
            body,
            HealthResponse {
                stt: false,
                tts: false,
                ocr: false,
            }
        );
    }
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test -p matrix-bot-inference health`

Expected: FAIL — compilation error, `cannot find function 'health' in this scope`.

- [ ] **Step 6: Write the health handler**

Append to `inference/src/routes/health.rs`, above the `#[cfg(test)]` block. `stt` and `tts` are hardcoded `false` because those capabilities do not exist yet; the STT and TTS plans replace each literal with a real presence check.

```rust
/// Reports which models are installed. `stt` and `tts` are hardcoded false
/// until those capabilities land; see the STT and TTS plans.
pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        stt: false,
        tts: false,
        ocr: crate::paddle::models_present(&state.config.ocr_model_dir),
    })
}
```

This references `crate::paddle::models_present`, which Task 2 creates. For this task only, add a temporary shim so the crate compiles. Create `inference/src/paddle.rs`:

```rust
use std::path::Path;

/// Whether every ONNX model OCR needs is present on disk.
///
/// Task 2 replaces this file wholesale with the module moved from the backend;
/// this stub exists so `/health` can be tested before the models arrive.
pub fn models_present(_dir: &Path) -> bool {
    false
}
```

- [ ] **Step 7: Write the router module**

Create `inference/src/routes/mod.rs`.

```rust
use axum::{extract::DefaultBodyLimit, routing::get, Router};
use std::sync::Arc;

use crate::config::InferenceConfig;

pub mod health;

/// Images and audio clips are the payloads here; 32 MB comfortably exceeds a
/// Matrix attachment while still refusing an unbounded upload.
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<InferenceConfig>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}
```

- [ ] **Step 8: Write the binary entrypoint**

Create `inference/src/main.rs`.

```rust
use std::sync::Arc;
use tracing::info;

mod config;
mod paddle;
mod routes;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,matrix_bot_inference=info".into()),
        )
        .init();

    let config = config::InferenceConfig::from_env();
    let port = config.port;

    let state = routes::AppState {
        config: Arc::new(config),
    };

    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    info!("Inference server listening on 0.0.0.0:{}", port);

    axum::serve(listener, routes::build_router(state)).await?;
    Ok(())
}
```

- [ ] **Step 9: Run the test to verify it passes**

Run: `cargo test -p matrix-bot-inference health`

Expected: PASS — one test.

- [ ] **Step 10: Verify the bot still resolves from the repo root**

Run: `cargo check`

Expected: succeeds and builds **only** `matrix-bot`, because `default-members` is unchanged. If the output mentions `matrix-bot-inference`, `default-members` was edited by mistake — revert that hunk.

- [ ] **Step 11: Format and check the whole workspace**

Run: `cargo fmt --all` then `cargo check --workspace`

Expected: no warnings.

- [ ] **Step 12: Commit**

```bash
git add Cargo.toml Cargo.lock inference/
git commit -m "feat: add the matrix-bot-inference workspace member

An Axum server that will host the local ONNX models. This commit is the
scaffold only: it serves GET /health, which reports every capability as
absent, and touches nothing in the backend.

default-members stays [\"backend\"] so bare \`cargo run\` from the repo root
still resolves to the bot.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 2: Move PaddleOCR into the inference crate and serve `POST /ocr`

**Files:**
- Modify: `inference/src/paddle.rs` (replace the Task 1 stub with the moved module), `inference/src/routes/mod.rs`, `inference/Cargo.toml`
- Create: `inference/src/routes/ocr.rs`
- Test: `inference/src/routes/ocr.rs` and the moved `#[cfg(test)]` block inside `inference/src/paddle.rs`

**Interfaces:**
- Consumes: `AppState`, `build_router` (Task 1).
- Produces:
  - `inference::paddle::run_ocr(image_bytes: Vec<u8>, model_dir: PathBuf) -> Result<OcrResult, OcrError>`
  - `inference::paddle::models_present(dir: &Path) -> bool`
  - `inference::paddle::{OcrResult, OcrTextBlock, OcrError}` — moved from `backend/src/modules/ocr/models.rs`
  - Route `POST /ocr`, raw image bytes in, body `{ "text": String, "blocks": [{ "text": String, "score": f32 }] }`

- [ ] **Step 1: Move the paddle module**

Replace the stub. This is a move, not a rewrite — the file's contents transfer unchanged except for the import rewrite in Step 2 and the addition in Step 4.

`inference/src/paddle.rs` already exists as the Task 1 stub, and plain `git mv` refuses to overwrite it. Delete the stub first so the move is a clean rename in history:

```bash
rm inference/src/paddle.rs
git mv backend/src/modules/ocr/paddle.rs inference/src/paddle.rs
```

- [ ] **Step 2: Move the OCR data types into the paddle module**

`backend/src/modules/ocr/models.rs` currently defines `OcrTextBlock`, `OcrResult`, `OcrError` and `OcrConfig`. The first three are needed by `paddle.rs` and must come along; `OcrConfig` stays in the backend and is reworked in Task 4.

Copy `OcrTextBlock`, `OcrResult` (with its `new` and `is_empty`) and `OcrError` into the top of `inference/src/paddle.rs`, then change the moved file's import line from

```rust
use super::models::{OcrError, OcrResult, OcrTextBlock};
```

to nothing — they are now defined in this file. No `Serialize` derive: the route in Step 10 maps `OcrResult` onto its own wire type, so the domain type stays free of serde.

```rust
use std::path::{Path, PathBuf};
use thiserror::Error;
use tracing::info;

/// A single detected text block from an image with its confidence score.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrTextBlock {
    pub text: String,
    pub score: f32,
}

/// The aggregated result of OCR recognition across all detected blocks.
#[derive(Debug, Clone, Default)]
pub struct OcrResult {
    pub blocks: Vec<OcrTextBlock>,
    pub combined_text: String,
}

impl OcrResult {
    pub fn new(blocks: Vec<OcrTextBlock>) -> Self {
        let combined_text = blocks
            .iter()
            .map(|b| b.text.trim())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        Self {
            blocks,
            combined_text,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.combined_text.trim().is_empty()
    }
}

/// Errors that can occur during OCR processing.
#[derive(Error, Debug)]
pub enum OcrError {
    #[error("Failed to decode image bytes: {0}")]
    ImageDecode(String),
    #[error("Missing OCR model file: {0}")]
    MissingModel(String),
    #[error("Failed to download OCR model: {0}")]
    ModelDownload(String),
    #[error("PaddleOCR inference error: {0}")]
    Inference(String),
    #[error("Worker thread execution error: {0}")]
    Join(String),
}
```

`OcrError::Disabled` is deliberately dropped. "Disabled" is a backend concern — the inference server either has the models or does not, and `models_present` answers that.

- [ ] **Step 3: Move the native dependencies**

Add to `inference/Cargo.toml` under `[dependencies]`:

```toml
image = "0.25"
ort = "=2.0.0-rc.10"
ort-sys = "=2.0.0-rc.10"
paddle-ocr-rs = "0.6.1"
reqwest = { version = "0.11", default-features = false, features = ["rustls-tls-webpki-roots"] }
```

`reqwest` is needed because the moved `ensure_models` downloads model files from Hugging Face.

Do **not** remove anything from `backend/Cargo.toml` yet — the backend still references `paddle` until Task 4. Removing it now breaks the build between commits.

- [ ] **Step 4: Write the failing test for `models_present`**

Add to the `#[cfg(test)]` block that came across with the file:

```rust
#[test]
fn test_models_present_is_false_when_the_directory_does_not_exist() {
    assert!(!models_present(Path::new("/nonexistent/ocr/models")));
}

#[test]
fn test_models_present_is_true_only_when_every_model_file_exists() {
    let dir = std::env::temp_dir().join(format!("ocr-present-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir should be creatable");

    // One file at a time: presence must require all three, not any.
    std::fs::write(dir.join(DET_MODEL_NAME), b"x").unwrap();
    assert!(!models_present(&dir));

    std::fs::write(dir.join(CLS_MODEL_NAME), b"x").unwrap();
    assert!(!models_present(&dir));

    std::fs::write(dir.join(REC_MODEL_NAME), b"x").unwrap();
    assert!(models_present(&dir));

    std::fs::remove_dir_all(&dir).ok();
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test -p matrix-bot-inference models_present`

Expected: FAIL — `models_present` in `paddle.rs` is still the Task 1 stub returning `false`, so the second test fails on its final assertion.

- [ ] **Step 6: Implement `models_present`**

Replace the Task 1 stub in `inference/src/paddle.rs` with the real check. It uses `resolve_model_dir`, which came across with the move, so a relative path behaves the same way it does during inference.

```rust
/// Whether every ONNX model OCR needs is present on disk.
///
/// All three are required: PaddleOCR runs detection, classification and
/// recognition in sequence, so a partial download is not a usable install.
pub fn models_present(dir: &Path) -> bool {
    let resolved = resolve_model_dir(dir);
    [DET_MODEL_NAME, CLS_MODEL_NAME, REC_MODEL_NAME]
        .iter()
        .all(|name| resolved.join(name).is_file())
}
```

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p matrix-bot-inference models_present`

Expected: PASS — two tests.

- [ ] **Step 8: Write the failing route test**

Create `inference/src/routes/ocr.rs` with the response type and its test. The handler follows in Step 10.

```rust
use axum::{body::Bytes, extract::State, http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use tracing::warn;

use super::AppState;

/// The OCR wire response. `text` is the blocks joined by newlines, which is
/// what every caller actually consumes; `blocks` carries the per-block scores
/// for callers that want them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrScanBody {
    pub text: String,
    pub blocks: Vec<OcrScanBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OcrScanBlock {
    pub text: String,
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::InferenceConfig;
    use std::sync::Arc;

    fn state_with_missing_models() -> AppState {
        AppState {
            config: Arc::new(InferenceConfig {
                port: 0,
                ocr_model_dir: std::path::PathBuf::from("/nonexistent/ocr/models"),
            }),
        }
    }

    #[tokio::test]
    async fn test_empty_body_is_rejected_as_bad_request() {
        let result = scan(State(state_with_missing_models()), Bytes::new()).await;

        let err = result.expect_err("an empty body should not be accepted");
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_missing_models_report_service_unavailable_not_a_crash() {
        let result = scan(State(state_with_missing_models()), Bytes::from_static(b"notanimage")).await;

        let err = result.expect_err("absent models should be an error");
        assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    }
}
```

- [ ] **Step 9: Run the test to verify it fails**

Run: `cargo test -p matrix-bot-inference routes::ocr`

Expected: FAIL — compilation error, `cannot find function 'scan' in this scope`.

- [ ] **Step 10: Write the route handler**

Append to `inference/src/routes/ocr.rs`, above the `#[cfg(test)]` block.

```rust
/// `POST /ocr` — raw image bytes in, recognised text out.
///
/// The body is the image exactly as the caller received it; the caller does no
/// decoding, which is what keeps the `image` crate out of the bot.
pub async fn scan(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<Json<OcrScanBody>, (StatusCode, String)> {
    if body.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Request body is empty; expected raw image bytes".to_string(),
        ));
    }

    if !crate::paddle::models_present(&state.config.ocr_model_dir) {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "OCR models are not installed on this inference server".to_string(),
        ));
    }

    let result = crate::paddle::run_ocr(body.to_vec(), state.config.ocr_model_dir.clone())
        .await
        .map_err(|e| {
            warn!("PaddleOCR inference failed: {:?}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(OcrScanBody {
        text: result.combined_text,
        blocks: result
            .blocks
            .into_iter()
            .map(|b| OcrScanBlock {
                text: b.text,
                score: b.score,
            })
            .collect(),
    }))
}
```

- [ ] **Step 11: Register the route**

In `inference/src/routes/mod.rs`, add the module declaration and the route.

```rust
use axum::{extract::DefaultBodyLimit, routing::{get, post}, Router};
```

```rust
pub mod health;
pub mod ocr;
```

```rust
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/ocr", post(ocr::scan))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}
```

- [ ] **Step 12: Run the tests to verify they pass**

Run: `cargo test -p matrix-bot-inference`

Expected: PASS — the health test, the two `models_present` tests, the two route tests, plus every test that came across inside `paddle.rs`.

- [ ] **Step 13: Confirm the backend is still broken in the expected way**

Run: `cargo check -p matrix-bot`

Expected: FAIL — `file not found for module 'paddle'` in `backend/src/modules/ocr/mod.rs`. This is correct; Task 4 fixes it. Note the error so the next reviewer is not surprised by a red tree between commits.

- [ ] **Step 14: Format and commit**

```bash
cargo fmt --all
git add inference/ backend/src/modules/ocr/paddle.rs Cargo.lock
git commit -m "feat: serve PaddleOCR from the inference crate

Moves paddle.rs and the OCR data types out of the backend and exposes them
as POST /ocr, taking raw image bytes and returning recognised text with
per-block scores. /health now reports real OCR model presence.

The backend does not compile at this commit -- it still references the module
that moved. The next commit rewires it.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 3: The `services/ocr` HTTP client

**Files:**
- Create: `backend/src/services/mod.rs`, `backend/src/services/ocr/mod.rs`
- Modify: `backend/src/lib.rs`
- Test: `backend/src/services/ocr/mod.rs` (in-file `#[cfg(test)]`)

**Interfaces:**
- Consumes: the `POST /ocr` contract from Task 2.
- Produces:
  - `services::ocr::OcrClient::new(base_url: Option<String>) -> OcrClient`
  - `OcrClient::is_configured(&self) -> bool`
  - `OcrClient::scan(&self, image_bytes: Vec<u8>) -> Result<OcrScan, OcrServiceError>`
  - `services::ocr::OcrScan { text: String, blocks: Vec<OcrScanBlock> }`
  - `services::ocr::OcrScanBlock { text: String, score: f32 }`
  - `services::ocr::OcrServiceError` with variants `NotConfigured`, `Unreachable(String)`, `Upstream { status: u16, message: String }`

`OcrScan` is a distinct type from `modules::ocr::OcrResult` on purpose: `services/` must not import from `modules/`, or the boundary this plan exists to create is inverted on its first use. Task 4 maps between them.

- [ ] **Step 1: Declare the services layer**

Create `backend/src/services/mod.rs`:

```rust
//! Adapters for capabilities provided outside this process.
//!
//! `modules/` is what the application does; `services/` is how external
//! capabilities are reached. Nothing in here may import from `modules/`.

pub mod ocr;
```

Add to `backend/src/lib.rs`, keeping the existing declarations in alphabetical order:

```rust
pub mod services;
```

- [ ] **Step 2: Write the failing tests**

Create `backend/src/services/ocr/mod.rs`. Types first, then tests; the implementation lands in Step 4.

The test server is a throwaway Axum app on an ephemeral port. `axum` and `tokio` are already backend dependencies, so this adds no crate.

```rust
use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;
use tracing::warn;

/// How long a single OCR call may take. PaddleOCR on a CPU is seconds, not
/// milliseconds, so this is generous; it exists to bound a hung connection,
/// not to police inference speed.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OcrScanBlock {
    pub text: String,
    pub score: f32,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct OcrScan {
    pub text: String,
    pub blocks: Vec<OcrScanBlock>,
}

#[derive(Debug, Error)]
pub enum OcrServiceError {
    #[error("OCR service is not configured")]
    NotConfigured,
    #[error("OCR service is unreachable: {0}")]
    Unreachable(String),
    #[error("OCR service returned {status}: {message}")]
    Upstream { status: u16, message: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::json;

    /// Starts a throwaway server on an ephemeral port and returns its base URL.
    /// Uses axum and tokio, both already backend dependencies, rather than
    /// adding a mock-HTTP crate.
    async fn spawn_server(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("ephemeral port should bind");
        let addr = listener.local_addr().expect("listener should have an address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.ok();
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn test_an_unconfigured_client_reports_not_configured() {
        let client = OcrClient::new(None);

        assert!(!client.is_configured());
        assert!(matches!(
            client.scan(vec![1, 2, 3]).await,
            Err(OcrServiceError::NotConfigured)
        ));
    }

    #[tokio::test]
    async fn test_a_successful_scan_is_parsed() {
        let app = Router::new().route(
            "/ocr",
            post(|| async {
                Json(json!({
                    "text": "SHAKE SHACK\nTotal $16.50",
                    "blocks": [
                        { "text": "SHAKE SHACK", "score": 0.98 },
                        { "text": "Total $16.50", "score": 0.99 }
                    ]
                }))
            }),
        );
        let base = spawn_server(app).await;
        let client = OcrClient::new(Some(base));

        let scan = client.scan(vec![1, 2, 3]).await.expect("scan should succeed");

        assert_eq!(scan.text, "SHAKE SHACK\nTotal $16.50");
        assert_eq!(scan.blocks.len(), 2);
        assert_eq!(scan.blocks[0].text, "SHAKE SHACK");
    }

    #[tokio::test]
    async fn test_an_upstream_error_carries_its_status_and_message() {
        let app = Router::new().route(
            "/ocr",
            post(|| async {
                (
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "OCR models are not installed on this inference server",
                )
            }),
        );
        let base = spawn_server(app).await;
        let client = OcrClient::new(Some(base));

        let err = client.scan(vec![1, 2, 3]).await.expect_err("503 should be an error");

        match err {
            OcrServiceError::Upstream { status, message } => {
                assert_eq!(status, 503);
                assert!(message.contains("not installed"));
            }
            other => panic!("expected an upstream error, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_an_unreachable_service_is_not_a_panic() {
        // Port 1 on loopback refuses immediately rather than hanging.
        let client = OcrClient::new(Some("http://127.0.0.1:1".to_string()));

        let err = client.scan(vec![1, 2, 3]).await.expect_err("a refused connection should error");

        assert!(matches!(err, OcrServiceError::Unreachable(_)));
    }

    #[tokio::test]
    async fn test_a_trailing_slash_in_the_base_url_does_not_double_up() {
        let app = Router::new().route(
            "/ocr",
            post(|| async { Json(json!({ "text": "ok", "blocks": [] })) }),
        );
        let base = spawn_server(app).await;
        let client = OcrClient::new(Some(format!("{}/", base)));

        let scan = client.scan(vec![1]).await.expect("a trailing slash should be tolerated");

        assert_eq!(scan.text, "ok");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p matrix-bot services::ocr`

Expected: FAIL — compilation error, `cannot find type 'OcrClient' in this scope`.

- [ ] **Step 4: Implement the client**

Insert above the `#[cfg(test)]` block in `backend/src/services/ocr/mod.rs`.

```rust
/// HTTP client for the inference server's OCR endpoint.
///
/// An absent `base_url` is a normal state, not an error condition: it means
/// OCR is not configured for this deployment, and every call reports
/// `NotConfigured` so the caller can degrade that one feature.
#[derive(Debug, Clone)]
pub struct OcrClient {
    base_url: Option<String>,
    http: reqwest::Client,
}

impl OcrClient {
    pub fn new(base_url: Option<String>) -> Self {
        let base_url = base_url
            .map(|u| u.trim().trim_end_matches('/').to_string())
            .filter(|u| !u.is_empty());

        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap_or_default();

        Self { base_url, http }
    }

    pub fn is_configured(&self) -> bool {
        self.base_url.is_some()
    }

    /// Sends the image bytes untouched and returns the recognised text.
    pub async fn scan(&self, image_bytes: Vec<u8>) -> Result<OcrScan, OcrServiceError> {
        let base = self
            .base_url
            .as_ref()
            .ok_or(OcrServiceError::NotConfigured)?;

        let response = self
            .http
            .post(format!("{}/ocr", base))
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(image_bytes)
            .send()
            .await
            .map_err(|e| {
                warn!("OCR service request failed: {}", e);
                OcrServiceError::Unreachable(e.to_string())
            })?;

        let status = response.status();
        if !status.is_success() {
            let message = response
                .text()
                .await
                .unwrap_or_else(|_| "no response body".to_string());
            return Err(OcrServiceError::Upstream {
                status: status.as_u16(),
                message,
            });
        }

        response
            .json::<OcrScan>()
            .await
            .map_err(|e| OcrServiceError::Upstream {
                status: status.as_u16(),
                message: format!("Malformed OCR response: {}", e),
            })
    }
}
```

`Default` is not derived on `reqwest::Client` in all versions; if `unwrap_or_default()` does not compile, replace it with `.expect("a reqwest client with only a timeout set is always constructible")`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p matrix-bot services::ocr`

Expected: PASS — five tests.

- [ ] **Step 6: Commit**

```bash
git add backend/src/services backend/src/lib.rs
git commit -m "feat: add the services/ocr HTTP client

First member of the services layer: modules/ is what the application does,
services/ is how external capabilities are reached. Nothing in services/
imports from modules/, so OcrScan is its own type rather than a reuse of
modules::ocr::OcrResult.

An absent base URL is a normal state reported as NotConfigured, not an
error, so a caller can degrade OCR alone.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 4: Cut both call sites over and drop the native dependencies

**Files:**
- Modify: `backend/src/modules/ocr/mod.rs`, `backend/src/modules/ocr/models.rs`, `backend/src/api/ocr.rs`, `backend/src/api/auth/types.rs`, `backend/src/bot/attachments.rs`, `backend/src/bot/handler.rs`, `backend/src/config.rs`, `backend/Cargo.toml`
- Test: the existing `#[cfg(test)]` blocks in each modified file

**Interfaces:**
- Consumes: `services::ocr::{OcrClient, OcrScan, OcrServiceError}` (Task 3).
- Produces: `modules::ocr::OcrConfig { enabled: bool, base_url: Option<String> }` — `model_dir` is gone.

- [ ] **Step 1: Rework `OcrConfig`**

In `backend/src/modules/ocr/models.rs`, delete `OcrTextBlock`, `OcrResult` and `OcrError` — they moved to the inference crate in Task 2 — and replace `OcrConfig`. Keep `OcrResult` and `OcrTextBlock` **only** as the shapes `modules/ocr` still uses internally for formatting; redefine them here without `Serialize`.

```rust
/// A single detected text block with its confidence score.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrTextBlock {
    pub text: String,
    pub score: f32,
}

/// OCR output as the business layer consumes it.
#[derive(Debug, Clone, Default)]
pub struct OcrResult {
    pub blocks: Vec<OcrTextBlock>,
    pub combined_text: String,
}

impl OcrResult {
    pub fn is_empty(&self) -> bool {
        self.combined_text.trim().is_empty()
    }
}

impl From<crate::services::ocr::OcrScan> for OcrResult {
    fn from(scan: crate::services::ocr::OcrScan) -> Self {
        Self {
            combined_text: scan.text,
            blocks: scan
                .blocks
                .into_iter()
                .map(|b| OcrTextBlock {
                    text: b.text,
                    score: b.score,
                })
                .collect(),
        }
    }
}

/// Configuration for the OCR module.
///
/// `base_url` absent means OCR is not available in this deployment; the
/// feature degrades and nothing else is affected.
#[derive(Debug, Clone, Default)]
pub struct OcrConfig {
    pub enabled: bool,
    pub base_url: Option<String>,
}
```

`OcrResult::new` is deleted: blocks are no longer joined locally, the service sends `text` already combined.

- [ ] **Step 2: Rewrite the module call site**

In `backend/src/modules/ocr/mod.rs`:

Remove `pub mod paddle;` and change the re-export line to drop `OcrError`:

```rust
pub use models::{OcrConfig, OcrResult, OcrTextBlock};
```

Change the signature of `process_image_event`: the `model_dir: &Path` parameter becomes `ocr: &crate::services::ocr::OcrClient`, and the `use std::path::Path;` import is removed.

Replace the `paddle::run_ocr` block with:

```rust
    let ocr_result: OcrResult = match ocr.scan(image_bytes.to_vec()).await {
        Ok(scan) => scan.into(),
        Err(crate::services::ocr::OcrServiceError::NotConfigured) => {
            return "⚠️ <b>图片处理不可用</b><br/>\
                    • <b>原因:</b> 本部署未配置文字识别服务<br/>\
                    • <b>建议:</b> 请直接输入文字，或联系管理员启用 OCR。"
                .to_string();
        }
        Err(err) => {
            warn!("OCR extraction failed for owner {}: {:?}", owner_id, err);
            return format!(
                "⚠️ <b>文字识别失败</b><br/>\
                 • <b>原因:</b> 图片文字识别遇到异常（{}）<br/>\
                 • <b>建议:</b> 请尝试重新拍摄清晰平整的图片，或直接输入文字。",
                html_escape(&err.to_string())
            );
        }
    };
```

The `NotConfigured` arm is the degradation the spec requires: it names the cause without pretending the image was unreadable.

- [ ] **Step 3: Rewrite the API call site**

In `backend/src/api/ocr.rs`, change `use crate::modules::ocr::paddle;` to nothing, and replace the `paddle::run_ocr` block with a call through the state's client. `extract_receipt_fields` and everything below it is untouched — it is offline heuristic parsing with no dependency on how the text was produced.

```rust
    let ocr_result = match state.ocr.scan(bytes).await {
        Ok(scan) => scan,
        Err(crate::services::ocr::OcrServiceError::NotConfigured) => {
            return Err(ApiError::service_unavailable(
                "OCR service is not configured on this server",
            ));
        }
        Err(err) => {
            warn!("OCR extraction failed: {:?}", err);
            return Err(ApiError::internal(format!("OCR extraction failed: {}", err)));
        }
    };

    let blocks: Vec<OcrBlockDto> = ocr_result
        .blocks
        .into_iter()
        .map(|b| OcrBlockDto {
            text: b.text,
            score: b.score,
        })
        .collect();

    let structured = extract_receipt_fields(&ocr_result.text);

    Ok(Json(OcrScanResponse {
        raw_text: ocr_result.text,
        blocks,
        structured: Some(structured),
    }))
```

No `#[utoipa::path]` annotation or `ToSchema` struct changes here, so `openapi.json` does not move. Step 9 verifies that.

- [ ] **Step 4: Carry the client on `ApiState`**

In `backend/src/api/auth/types.rs`, add a field beside the existing `ocr_config`:

```rust
    pub ocr: crate::services::ocr::OcrClient,
```

Build it wherever `ocr_config` is built — in `from_config`, construct it from the config's base URL; in the test constructors at lines ~197 and ~250, construct it with `OcrClient::new(None)`, which is the unconfigured state and keeps those tests offline.

```rust
    ocr: crate::services::ocr::OcrClient::new(config.ocr_base_url.clone()),
```

- [ ] **Step 5: Thread the client through the attachment path**

`Attachment<'a>` is a **borrowing** enum — `backend/src/bot/attachments.rs:36` holds `model_dir: &'a Path`. Keep it borrowing rather than switching to an owned field, so the `'a` parameter and every signature that carries it stay as they are. This is a one-for-one type substitution:

```rust
    Image {
        content: ImageMessageEventContent,
        ocr: &'a crate::services::ocr::OcrClient,
    },
```

Change the constructor parameter at line 47 from `ocr_model_dir: &'a Path` to `ocr: &'a crate::services::ocr::OcrClient`, the field init at line 53 to `ocr`, and the `from_msgtype` caller at line 219/223 the same way. The `use std::path::Path;` import goes if nothing else in the file uses it.

In `backend/src/bot/handler.rs`, line 39 currently does `let ocr_model_dir = config.ocr_model_dir.clone();` and line 52 re-clones it into the spawned task. `OcrClient` derives `Clone`, so this is the same shape:

```rust
    let ocr_client = crate::services::ocr::OcrClient::new(config.ocr_base_url.clone());
```

```rust
        let ocr_client = ocr_client.clone();
```

and the call at line 80 passes `&ocr_client` instead of `&ocr_model_dir`.

In `run_module` change the match arm:

```rust
            Self::Image { content, ocr } => {
                crate::modules::ocr::process_image_event(
                    db,
                    ai,
                    broadcaster,
                    room,
                    owner_id,
                    Some(&content.body),
                    media_bytes,
                    ocr,
                    Some(reporter),
                )
                .await
            }
```

In `backend/src/bot/handler.rs`, wherever the image branch currently supplies a model directory, supply an `OcrClient` built from the config instead.

- [ ] **Step 6: Replace the config fields**

In `backend/src/config.rs`:

Delete the `ocr_model_dir` field and its `env::var("OCR_MODEL_DIR")` block. Add:

```rust
    /// Base URL of the inference server's OCR endpoint.
    ///
    /// Absent means OCR is unavailable in this deployment: the feature
    /// degrades with a message, and nothing else is affected.
    pub ocr_base_url: Option<String>,
```

```rust
        let ocr_base_url = env::var("OCR_BASE_URL")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
```

Add `ocr_base_url` to every `Config` literal in the file, including the two test fixtures near lines 421 and 463, where it should be `None`.

- [ ] **Step 7: Drop the dependencies**

In `backend/Cargo.toml`, delete these two lines:

```toml
image = "0.25"
paddle-ocr-rs = "0.6.1"
```

Leave `ort`, `ort-sys` and `sherpa-onnx` in place — `sherpa-onnx` is still used by `modules/voice` and `modules/tts`, and the `ort` pins exist to unify onnxruntime across it. The TTS plan removes all three.

- [ ] **Step 8: Run the full backend suite**

Run: `cargo test -p matrix-bot`

Expected: PASS. `test_format_ocr_turn_response` in `modules/ocr/mod.rs` builds its `OcrResult` by struct literal, which the Step 1 redefinition still allows, so it compiles unchanged. `OcrResult::new` had exactly one caller in the whole backend — inside `paddle.rs`, which moved — so deleting it strands nothing.

- [ ] **Step 9: Verify the OpenAPI spec did not move**

Run:

```bash
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
```

Expected: exit 0, no diff. If there is a diff, an annotated type changed unintentionally — investigate before committing rather than regenerating `api.ts` to match.

- [ ] **Step 10: Confirm the dependencies are gone**

Run:

```bash
cargo tree -p matrix-bot --depth 1 | grep -E "paddle-ocr-rs|^.*image v" || echo "both removed"
```

Expected: `both removed`.

- [ ] **Step 11: Format, check, commit**

```bash
cargo fmt --all && cargo check --workspace
git add backend/
git commit -m "refactor: call OCR over HTTP and drop image + paddle-ocr-rs

modules/ocr keeps every line of its business logic -- the AI turn, the
prompt, the Matrix reply -- and only its call to paddle::run_ocr becomes a
network call. api/ocr.rs changes the same way; extract_receipt_fields is
offline heuristic parsing and is untouched.

An unset OCR_BASE_URL degrades OCR alone with a message naming the cause,
and leaves the rest of the bot working.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

---

## Task 5: Ship it — Docker, Compose, CI, docs

**Files:**
- Create: `Dockerfile.inference`
- Modify: `docker-compose.yml`, `.github/workflows/ci.yml`, `.env.example`, `README.md`, `CLAUDE.md`, `AGENTS.md`

**Interfaces:**
- Consumes: everything above.
- Produces: a two-container deployment, and a CI layout where the backend jobs no longer build native ML dependencies.

- [ ] **Step 1: Write the inference Dockerfile**

Create `Dockerfile.inference`. No `cmake` — that was for `opusic-sys`, which does not arrive until the TTS plan; add it in that plan, not now.

```dockerfile
# syntax=docker/dockerfile:1

# ---- Stage 1: build ---------------------------------------------------------
FROM rust:1-bookworm AS build

RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY backend/Cargo.toml ./backend/Cargo.toml
COPY inference/Cargo.toml ./inference/Cargo.toml

# Dependency layer: dummy sources for both members so the workspace resolves,
# built for the inference bin only.
RUN mkdir -p backend/src backend/src/bin inference/src && \
    echo "pub fn dummy() {}" > backend/src/lib.rs && \
    echo "fn main() {}" > backend/src/main.rs && \
    echo "fn main() {}" > backend/src/bin/generate_openapi.rs && \
    echo "fn main() {}" > inference/src/main.rs && \
    mkdir -p backend/web/dist && touch backend/web/dist/index.html && \
    cargo build --release -p matrix-bot-inference && \
    rm -rf inference/src target/release/deps/matrix_bot_inference*

COPY backend ./backend
COPY inference ./inference

RUN cargo build --release -p matrix-bot-inference

# ---- Stage 2: runtime -------------------------------------------------------
FROM debian:bookworm-slim

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 curl \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd -g 10001 app \
 && useradd -u 10001 -g app -s /usr/sbin/nologin app

WORKDIR /app

COPY --from=build /src/target/release/matrix-bot-inference /usr/local/bin/matrix-bot-inference

# Models are downloaded on first use into this directory, which is a volume so
# the download survives a container replacement.
RUN mkdir -p /app/data && chown -R app:app /app
USER app

ENV INFERENCE_PORT=8090 \
    OCR_MODEL_DIR=/app/data/ocr_models

EXPOSE 8090
VOLUME ["/app/data"]

HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -f http://127.0.0.1:8090/health || exit 1

CMD ["matrix-bot-inference"]
```

- [ ] **Step 2: Add the compose service**

In `docker-compose.yml`, add to the `matrix-bot` service's `environment` list:

```yaml
      - OCR_BASE_URL=${OCR_BASE_URL:-http://inference:8090}
```

Add a new top-level service. It is **not** published to the host — the bot reaches it by service name on compose's default network.

```yaml
  # Local ONNX model inference, reached only from the bot over the compose
  # network. Deliberately not published to the host: it has no authentication,
  # which is safe on an internal network and is not safe anywhere else.
  inference:
    build:
      context: .
      dockerfile: Dockerfile.inference
    image: matrix-bot-inference:latest
    container_name: matrix-bot-inference
    restart: unless-stopped
    environment:
      - INFERENCE_PORT=8090
      - OCR_MODEL_DIR=/app/data/ocr_models
      - RUST_LOG=info,matrix_bot_inference=info
    volumes:
      - inference-data:/app/data
    healthcheck:
      test: ["CMD", "curl", "-f", "http://127.0.0.1:8090/health"]
      interval: 30s
      timeout: 5s
      retries: 3
      start_period: 10s
```

Add the volume:

```yaml
volumes:
  bot-data:
    name: matrix-bot-data
  inference-data:
    name: matrix-bot-inference-data
  postgres-data:
    name: matrix-bot-postgres-data
```

- [ ] **Step 3: Scope the backend CI jobs to the bot crate**

This is the step that actually delivers the CI benefit. Both backend jobs currently run `cargo test --workspace`, which would pull the inference crate — and its native dependencies — straight back into them.

In `.github/workflows/ci.yml`, in the `backend` job, change:

```yaml
      - name: Run backend tests & check targets
        run: cargo test --workspace --all-targets
```

to:

```yaml
      # Scoped to the bot crate. `--workspace` would drag the inference crate's
      # native ML dependencies back into this job, which is the cost this
      # split exists to remove.
      - name: Run backend tests & check targets
        run: cargo test -p matrix-bot --all-targets
```

In the `backend-postgres` job, change `cargo test --workspace` to `cargo test -p matrix-bot` with the same comment.

Leave both sherpa-onnx blocks in place for now: `modules/voice` and `modules/tts` still depend on `sherpa-onnx`, so the backend jobs still need it. The STT and TTS plans delete those blocks.

- [ ] **Step 4: Add the inference CI job**

Append a job to `.github/workflows/ci.yml`:

```yaml
  inference:
    name: Inference CI (check, test, format)
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt

      - uses: Swatinem/rust-cache@v2
        with:
          prefix-key: "v2-inference"

      - name: Format check
        run: cargo fmt --all -- --check

      - name: Run inference tests
        run: cargo test -p matrix-bot-inference --all-targets
```

- [ ] **Step 5: Run the CI commands locally**

```bash
cargo fmt --all -- --check
cargo test -p matrix-bot --all-targets
cargo test -p matrix-bot-inference --all-targets
```

Expected: all three succeed.

- [ ] **Step 6: Document the new environment variable**

In `.env.example`, replace the `OCR_MODEL_DIR` entry with:

```bash
# Base URL of the inference server that runs OCR. Under docker compose this is
# the service name. Leave it unset to run without OCR: image messages then
# answer with a message saying so, and nothing else is affected.
OCR_BASE_URL=http://inference:8090

# Read by the inference container, not the bot.
OCR_MODEL_DIR=/app/data/ocr_models
```

- [ ] **Step 7: Update `CLAUDE.md`**

Under "Repository Layout", add `inference/` to the member list. Then add this section immediately after "Single process, two front doors":

```markdown
### Two crates, one deployment

The workspace has two members. `backend/` builds `matrix-bot`; `inference/`
builds `matrix-bot-inference`, an Axum server that hosts the local ONNX
models. `default-members = ["backend"]` keeps bare `cargo run` meaning the
bot, and `cargo test -p matrix-bot` is the everyday suite —
`cargo test --workspace` additionally builds the crate carrying the native ML
dependencies, which is why CI scopes the backend jobs explicitly.

OCR reaches the inference server over HTTP via `services/ocr`. An unset
`OCR_BASE_URL`, or an unreachable server, degrades image handling alone: the
bot still boots, and `docker run` with a volume still yields a working bot
without OCR.

`backend/src/services/` is the boundary this split created. `modules/` is what
the application does; `services/` is how external capabilities are reached.
Nothing in `services/` may import from `modules/` — `services::ocr::OcrScan`
is a separate type from `modules::ocr::OcrResult` for exactly that reason,
with a `From` impl in the module doing the mapping.
```

- [ ] **Step 8: Update `AGENTS.md`**

Add to the conventions list:

```markdown
- **`services/` never imports from `modules/`.** `modules/` is what the
  application does; `services/` is how external capabilities are reached. A
  wire type and a domain type that happen to have the same fields stay
  separate, with a `From` impl in the module. Inverting this collapses the
  boundary on first use.
- **The 500-line limit applies to `inference/src/` too.**
```

- [ ] **Step 9: Update `README.md`**

In the deployment section, note that `docker compose up` now starts two containers, and that a bare `docker run` of `matrix-bot` still works without OCR.

- [ ] **Step 10: Commit**

```bash
git add Dockerfile.inference docker-compose.yml .github/workflows/ci.yml \
        .env.example README.md CLAUDE.md AGENTS.md
git commit -m "ci: build and deploy the inference crate separately

Adds Dockerfile.inference and a compose service on the internal network, not
published to the host.

Scopes both backend CI jobs to -p matrix-bot. They ran --workspace, which
would have pulled the inference crate's native dependencies straight back
into them and cancelled the benefit of the split.

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>"
```

- [ ] **Step 11: End-to-end verification**

```bash
docker compose build
docker compose up -d
curl -f http://localhost:8080/api/v1/health
docker compose exec matrix-bot curl -f http://inference:8090/health
```

Expected: the bot is healthy, and the inference health body reports `{"stt":false,"tts":false,"ocr":false}` on a fresh volume — models download on first use. Send an image to the bot in Matrix and confirm the OCR reply arrives.

Then confirm degradation:

```bash
docker compose stop inference
```

Send another image. Expected: the "识别失败" message, and `!todo list` still works.

---

## Plan sequence

This is plan 1 of 4. The rest are written after this one lands, so each is informed by what the first actually cost.

| Plan | Scope | Depends on |
|---|---|---|
| **1 — this plan** | Scaffold + OCR + delivery | — |
| 2 | STT: `sherpa.rs`, `audio.rs`, `/v1/audio/transcriptions`, `services/stt`. Drops `sherpa-onnx`, `symphonia`, `opus-decoder`, and one sherpa CI block. | Plan 1 |
| 3 | TTS: `engine.rs`, `opus.rs`, `text.rs`, `/v1/audio/speech` with the two extension headers, `services/tts`, the `tts_voice` migration and its frontend changes, then the ElevenLabs dialect. Drops `opus`, `ort`, `ort-sys`, `cmake`, and the last sherpa CI block. | Plan 1 |
| 4 | `services/llm` extracted from `modules/ai/client.rs`. Pure move. | Nothing — may land at any time |

Plan 4 is independent of the others and is the cheapest way to make the boundary visible early if that is wanted before the hard work.
