# matrix-bot-inference 🧠

Local, offline AI inference server for [`matrix-bot`](../README.md). Built with [Axum](https://github.com/tokio-rs/axum), [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx), [ort](https://github.com/pykeio/ort) (ONNX Runtime), and [paddle-ocr-rs](https://github.com/hiroi-sora/paddle-ocr-rs).

It provides three core capabilities:
- 🔍 **OCR (Optical Character Recognition)**: Local text recognition from images using PaddleOCR (detection, classification, and recognition models).
- 🎙️ **STT (Speech-to-Text)**: High-speed transcription for voice notes and audio clips using SenseVoice Small via sherpa-onnx.
- 🗣️ **TTS (Text-to-Speech)**: Expressive speech synthesis using Kokoro multi-lingual v1.1 (103 voices, English & Mandarin, with Chinese text normalization FSTs), outputting native Matrix voice bubbles as RFC 7845 Ogg Opus.

---

## 📁 Repository Layout

```text
inference/
├── Cargo.toml
├── README.md
├── models/                     # Model weights (gitignored except .gitkeep)
│   ├── .gitignore
│   ├── ocr/                   # PaddleOCR ONNX models
│   ├── stt/                   # SenseVoice ONNX model + tokens
│   └── tts/                   # Kokoro TTS model, voices, lexicons, and FSTs
└── src/
    ├── main.rs                # Server entry point & async task orchestration
    ├── config.rs              # Runtime configuration & environment loading
    ├── download.rs            # Resilient chunked model downloading with idle timeouts
    ├── provision.rs           # Background concurrent model auto-provisioning
    ├── test_support.rs        # Test fixtures & ephemeral test HTTP servers
    ├── api/                   # HTTP routing & API handlers
    │   ├── mod.rs             # Axum router & /health endpoint
    │   ├── ocr.rs             # POST /ocr
    │   ├── stt.rs             # POST /v1/audio/transcriptions
    │   ├── tts.rs             # POST /v1/audio/speech
    │   └── embeddings.rs      # POST /v1/embeddings
    ├── ocr/                   # OCR inference & model resolution
    │   ├── mod.rs             # Module exports
    │   └── paddle.rs          # PaddleOCR ONNX engine
    ├── stt/                   # Speech-to-text inference & audio decoding
    │   ├── mod.rs             # Module exports
    │   ├── audio.rs           # Audio decoding (WAV, MP3, AAC, FLAC, Ogg Opus) to 16kHz mono PCM
    │   └── sherpa.rs          # Sherpa-onnx SenseVoice offline recognizer
    ├── embeddings/            # Text embeddings (multilingual-e5-small)
    │   ├── mod.rs             # Constants, model-presence check
    │   ├── engine.rs          # Lazy one-time load, FIFO-serialised access
    │   ├── model.rs           # ONNX session + tokenizer, batching
    │   ├── pooling.rs         # Masked mean-pool + L2 normalisation
    │   └── provision.rs       # Pinned, SHA-256-verified model download
    └── tts/                   # Text-to-speech synthesis & audio encoding
        ├── mod.rs             # Module exports
        ├── kokoro.rs          # Kokoro multi-language TTS engine
        └── opus.rs            # RFC 7845 Ogg Opus audio encoder
```

---

## 🚀 Running Locally

### Option 1: Native (Cargo)

Run directly from the repository root:

```bash
cargo run -p matrix-bot-inference
```

- Server listens by default on `http://0.0.0.0:8090`.
- On boot, missing models are downloaded in the background concurrently (see [Model Auto-Provisioning](#-model-auto-provisioning)).

### Option 2: Docker Compose

Start the containerized inference service:

```bash
docker compose up inference
```

- Runs inside a lightweight Debian environment (`Dockerfile.inference`).
- Model files persist across container restarts in the `inference-data` Docker volume.

---

## 📡 API Reference

All routes are bounded by a 32 MB upload limit.

### 1. Health Check
`GET /health`

Returns the installation status of each capability's model:
```json
{
  "stt": true,
  "tts": true,
  "ocr": true,
  "embeddings": true
}
```
*Note: Responds immediately even while background provisioning is downloading models.*

---

### 2. Optical Character Recognition
`POST /ocr`

Takes raw image bytes in the request body (PNG, JPEG, WebP, etc.).

**Response:**
```json
{
  "text": "Extracted text content\nLine 2",
  "blocks": [
    { "text": "Extracted text content", "score": 0.98 },
    { "text": "Line 2", "score": 0.95 }
  ]
}
```

---

### 3. Speech-to-Text (OpenAI Compatible)
`POST /v1/audio/transcriptions`

Accepts `multipart/form-data`:
- `file` *(required)*: Audio file bytes (Ogg Opus, WAV, MP3, AAC, FLAC).
- `language` *(optional)*: `auto`, `zh`, `en`, `ja`, `ko`, `yue`.
- `response_format` *(optional)*: `json` (default) or `verbose_json`.

**Response (`json`):**
```json
{
  "text": "Transcribed speech content."
}
```

**Response (`verbose_json`):**
```json
{
  "task": "transcribe",
  "language": "en",
  "duration": 2.45,
  "text": "Transcribed speech content."
}
```

---

### 4. Text-to-Speech (OpenAI Compatible)
`POST /v1/audio/speech`

Accepts JSON:
```json
{
  "input": "Hello world, this is a test.",
  "voice": "0",
  "response_format": "opus"
}
```

- `voice`: Kokoro speaker ID (`0` to `102`).
- `input`: Text to synthesize (up to 4,096 characters).

**Response:**
- Binary `audio/ogg` stream encoded in Opus.
- `x-duration-ms`: Total duration in milliseconds.
- `x-waveform`: Proportional 100-bucket peak amplitude waveform array for Matrix voice messages.

---

### 5. Embeddings (OpenAI Compatible)
`POST /v1/embeddings`

Accepts JSON:
```json
{
  "input": ["passage: The cat sits on the mat", "passage: 猫坐在垫子上"],
  "model": "multilingual-e5-small"
}
```

- `input`: a string or an array of up to 128 strings, each up to 20,000 characters. Text beyond 512 tokens is truncated.
- `model` *(optional)*: must be `multilingual-e5-small`; any other name is a `400`, because vectors from different models are not comparable.
- **The server adds no prefix.** e5 expects `query: ` on search queries and `passage: ` on stored text; the backend applies them.
- **Response:** `{ "object": "list", "data": [{ "index": 0, "embedding": [384 floats, L2-normalised] }], "model", "usage" }`.
- `503` while the model is still downloading, or installed but unloadable (the cause is in the server log).

---

## 📦 Model Auto-Provisioning

Models download automatically on first run to the directories configured under `models/`:

| Capability | Models & Assets | Size | Source |
|---|---|---|---|
| **OCR** | `ch_PP-OCRv4_det_infer.onnx`<br>`ch_ppocr_mobile_v2.0_cls_infer.onnx`<br>`ch_PP-OCRv4_rec_infer.onnx` | ~16 MB | [RapidOCR Hugging Face](https://huggingface.co/SWHL/RapidOCR) |
| **STT** | `model.int8.onnx`<br>`tokens.txt` | ~230 MB | [SenseVoice Small Hugging Face](https://huggingface.co/csukuangfj/sherpa-onnx-sense-voice-zh-en-ja-ko-yue-2024-07-17) |
| **TTS** | `kokoro-multi-lang-v1_1.tar.bz2` (ONNX, voices, lexicons, FSTs) | ~311 MB | [Sherpa-onnx GitHub Releases](https://github.com/k2-fsa/sherpa-onnx/releases) |
| **Embeddings** | `model.onnx` (int8 `multilingual-e5-small`)<br>`tokenizer.json` | ~135 MB | [Xenova/multilingual-e5-small Hugging Face](https://huggingface.co/Xenova/multilingual-e5-small), pinned to a commit |

- **Resilient**: Downloads stream into `.tmp` files with an idle timeout and are renamed on completion, preventing corrupt or partial models from ever loading.
- **Verified**: the embedding files are checked against a SHA-256 pinned in `embeddings/provision.rs` before they are installed; a mismatch is retried, never loaded. Changing the model means changing the revision, both hashes, and the backend's `EMBEDDING_MODEL`.
- **Independent**: OCR, STT, TTS, and embeddings provision concurrently; a retry or network hiccup in one capability never blocks the others.

---

## ⚙️ Configuration

Environment variables (read at boot):

| Variable | Default | Description |
|---|---|---|
| `INFERENCE_PORT` | `8090` | Port for the Axum HTTP server to bind. |
| `OCR_MODEL_DIR` | `models/ocr` | Directory for PaddleOCR ONNX models. |
| `VOICE_MODEL_DIR` | `models/stt` | Directory for SenseVoice STT ONNX files. |
| `TTS_MODEL_DIR` | `models/tts` | Directory for Kokoro TTS unpacked model assets. |
| `EMBEDDING_MODEL_DIR` | `models/embeddings` | Directory for the e5 `model.onnx` + `tokenizer.json`. Keep it on the data volume (the container defaults do) so the ~135 MB download is not repeated on every restart. |

---

## 🧪 Testing

Run all unit and integration tests:

```bash
cargo test -p matrix-bot-inference --all-targets
```

All HTTP request and response structures are validated against shared JSON contracts in `contracts/`.
