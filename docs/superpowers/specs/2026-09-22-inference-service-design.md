# Extracting local model inference into a second crate

**Date:** 2026-09-22
**Status:** Approved for planning
**Branch:** `refactor/inference-service`

## Problem

Three ONNX models — PaddleOCR, SenseVoiceSmall (STT) and Kokoro (TTS) — run
in-process inside `matrix-bot`. They are the only consumers of nine
dependencies, and they impose costs on every part of the project that has
nothing to do with them:

- `backend/Cargo.toml` carries `image`, `ort`, `ort-sys`, `paddle-ocr-rs`,
  `sherpa-onnx`, `ogg`, `opus`, `opus-decoder` and `symphonia`.
- The Docker build stage needs `cmake`, because `opusic-sys` compiles a
  vendored libopus from source. The runtime stage ships
  `libsherpa-onnx-c-api.so` and `libonnxruntime.so`.
- Windows development requires copying sherpa's `onnxruntime.dll` into
  `target/debug/deps/`; a version mismatch there dies with an access
  violation rather than a clean error.
- CI downloads a sherpa-onnx archive. Two of the five commits preceding this
  branch (`619e7e0`, `1f5f9bf`) exist only to stop that download breaking
  builds.

Kokoro alone is 311 MB. A contributor who pulls the branch to change a todo
command pays all of the above.

Separately, `modules/ai/client.rs` is a 630-line OpenAI-compatible HTTP
transport living inside a business module whose handler is 1209 lines of
conversation and tool-loop logic. Those are two different concerns sharing a
folder.

## Goals

1. Remove every native ML dependency, and `cmake`, from the crate that builds
   the bot.
2. Make STT and TTS work against any OpenAI-compatible provider — and TTS
   additionally against ElevenLabs — so a deployment can choose local models, a
   cloud API, or a mix, by config alone.
3. Establish a `modules/` (what the application does) versus `services/` (how
   external capabilities are provided) boundary, with `services/llm` extracted
   from `modules/ai`.
4. Keep `docker run` with a volume producing a working bot.

## Non-goals

- A per-vendor **LLM** provider abstraction. `ai/client.rs` already takes a
  configurable `base_url`, which reaches xAI, Ollama, llama.cpp, vLLM,
  LM Studio, OpenRouter, and Gemini and Anthropic via their compatibility
  endpoints. `services/llm` is a **move**, not a redesign. Adding a provider
  enum there would be the speculative abstraction `CLAUDE.md` forbids.

  This does not contradict the TTS dialect below. For LLMs, `base_url` genuinely
  suffices; for TTS it genuinely does not, because ElevenLabs puts the voice in
  the URL path. Different facts, different answers.
- Moving `ocr`, `voice` or `tts` out of `modules/`. They are business modules
  (see "What stays" below).
- Splitting the three models into separate containers. See "Known unfixed
  risk".
- Solving OCR portability. No OpenAI-compatible OCR standard exists.

## Workspace shape

```
matrix-hello-bot/
├── Cargo.toml          members = ["backend", "inference"]
│                       default-members = ["backend"]
├── backend/            crate matrix-bot            (bot + API + web, no ML)
├── inference/          crate matrix-bot-inference  (NEW: Axum model server)
└── frontend/
```

`default-members` stays `["backend"]` so bare `cargo run` from the repo root
still resolves to the bot. `CLAUDE.md` already flags this as the case to watch
— *"Keep both if you add a workspace member or a third binary."* Adding
`inference` is that case.

## The modules / services boundary

```
backend/src/
├── modules/                      WHAT the application does
│   ├── ai/        client.rs      → services/llm
│   ├── ocr/       paddle.rs      → inference crate
│   ├── voice/     sherpa.rs, audio.rs
│   ├── tts/       engine.rs, opus.rs, text.rs, compute_waveform
│   └── …unchanged
└── services/                     HOW external capabilities are provided
    ├── llm/       moved from modules/ai/client.rs
    ├── stt/       OpenAI-compatible client
    ├── tts/       OpenAI-compatible client
    └── ocr/       client for our own route
```

### What stays in `modules/`

`ocr`, `voice` and `tts` are **not** media services and do not move. Each is a
business module whose inference call is one step of a larger flow:

```
ocr/mod.rs::process_image_event
    ├── run OCR on the bytes              ← adapter  (becomes a network call)
    ├── ai::handler::run_ai_turn_with_stage  ← business
    └── ai::handler::format_response         ← business
```

`voice/mod.rs` has the same shape. `tts/mod.rs` owns per-user voice resolution
and the Matrix upload. Moving these wholesale into a services layer would push
Matrix reply formatting and LLM orchestration into infrastructure.

Note also that the `modules/finance → OCR` edge sketched during discussion does
not exist. OCR feeds the AI turn, not finance.

### Three sibling clients, not one `ai_media`

STT, TTS and OCR share no types, no error domain and no protocol. They get
three independent client modules under `services/`, not a shared
`client.rs`/`config.rs`/`error.rs`. Grouping them into one **container** is an
ops decision and is unaffected; grouping them into one Rust module would invent
coupling that does not exist.

## Protocol

STT and TTS speak the OpenAI audio API so any compatible provider is a drop-in.
OCR has no standard and uses our own route.

| Route | Request | Response |
|---|---|---|
| `POST {STT_BASE_URL}/audio/transcriptions` | multipart: `file`, `model`, `language`, `response_format=json` | `{ text }` |
| `POST {TTS_BASE_URL}/audio/speech` | JSON `{ model, input, voice, response_format: "opus" }` | raw Ogg Opus bytes |
| `POST {OCR_BASE_URL}/ocr` | raw image bytes | `{ text, blocks: [{ text, confidence, box }] }` |
| `GET /health` | — | `{ stt: bool, tts: bool, ocr: bool }` per-model presence |

All three carry `Authorization: Bearer` when an API key is configured.

### STT asks for `json`, and the duration comes from Matrix

The client requests `response_format=json`, the one format every
OpenAI-compatible transcription model accepts: OpenAI's `gpt-4o-transcribe`
and `gpt-4o-mini-transcribe` reject `verbose_json`, so asking for it would make
OpenAI's recommended models fail on every voice note. `json` carries text only.
The voice note's duration, which the AI prompt mentions, comes from the Matrix
event's own `AudioInfo.duration`, set by the sender's client. A provider that
includes `language` or `duration` anyway is honoured. The bundled server still
serves `verbose_json` for any other client.

*Revised during Plan 2 execution; this section originally specified
`verbose_json`.*

### TTS returns bytes, and the metadata gap

`/audio/speech` returns raw audio with no metadata. Matrix voice messages need
a duration and a waveform for `AttachmentInfo::Voice`. Resolution:

- The response body is raw Ogg Opus, per the standard.
- `matrix-bot-inference` **additionally** sets `X-Duration-Ms` and
  `X-Waveform` response headers. This is a pure extension: third-party
  providers omit them, which is legal and invisible to them.
- When the headers are absent, the bot reads the duration from the Ogg granule
  position of the final page (pure-Rust `ogg` crate, no native build) and sends
  `waveform: None`. `AttachmentInfo::Voice` already accepts `Option`, so a
  client still renders a voice bubble — without amplitude bars.

Requesting `response_format: "opus"` is what keeps the voice bubble working;
`CLAUDE.md` records that Ogg Opus rather than WAV is what makes a client draw
one.

## Provider dialects

Most providers are reachable by URL alone. ElevenLabs is not, and differs on
four axes at once:

| | OpenAI | ElevenLabs |
|---|---|---|
| Auth header | `Authorization: Bearer` | `xi-api-key` |
| Endpoint | `/v1/audio/speech` | `/v1/text-to-speech/{voice_id}` |
| Voice | body field `voice` | in the URL path |
| Body | `{ input, model }` | `{ text, model_id }` |

A base URL cannot reconcile a voice that lives in the path with one that lives
in the body, so `services/tts` carries a dialect:

```
TTS_DIALECT=openai      # default
TTS_DIALECT=elevenlabs
```

The dialect selects the auth header, the URL shape and the body field names.
Nothing else in the design changes: the response is still raw audio, the
extension headers are still absent from third parties, and the Ogg granule
fallback still supplies the duration. `TTS_VOICE` carries a voice id rather
than a name under the ElevenLabs dialect, which the `Option<String>` column
already accommodates.

ElevenLabs must be asked for an Opus output format. If a chosen provider
cannot emit Ogg Opus, the voice bubble degrades to a plain audio attachment;
that is a provider choice, not a code path to add.

**STT ships with no dialect.** Every STT provider worth pointing at today —
OpenAI, Groq, speaches, LocalAI — is already OpenAI-compatible, so a dialect
enum with one member would be speculative. The same mechanism extends to
`services/stt` the day a non-compatible provider is wanted; ElevenLabs Scribe
is the known candidate.

## Configuration

One `INFERENCE_URL` would prevent mixing providers. Config is per capability,
mirroring the `base_url` / `api_key` / `model` shape `ai/client.rs` already
established:

```
STT_BASE_URL   STT_API_KEY   STT_MODEL
TTS_BASE_URL   TTS_API_KEY   TTS_MODEL   TTS_VOICE   TTS_DIALECT
OCR_BASE_URL
```

Worked examples:

```bash
# Local Kokoro via the inference crate (default)
TTS_BASE_URL=http://inference:8090/v1

# OpenAI — URL, key, model, voice. Nothing else.
TTS_BASE_URL=https://api.openai.com/v1
TTS_MODEL=tts-1
TTS_VOICE=alloy

# ElevenLabs — same four, plus the dialect
TTS_BASE_URL=https://api.elevenlabs.io/v1
TTS_DIALECT=elevenlabs
TTS_MODEL=eleven_multilingual_v2
TTS_VOICE=21m00Tcm4TlvDq8ikWAM

# Groq for STT, local for everything else
STT_BASE_URL=https://api.groq.com/openai/v1
STT_MODEL=whisper-large-v3
```

All default to the local `inference` service when present. Any one may point
elsewhere independently — local STT with cloud TTS is config, not code. After
this change `services/llm`, `services/stt` and `services/tts` are the same
shape three times.

## Degradation

A base URL that is unset, or a service that is unreachable, makes that one
feature answer with a plain "unavailable" message — the same shape
`TTS_ENABLED=false` produces today. Nothing else is affected: todo, finance,
AI chat, reminders and the web UI are untouched, and `docker run` with a volume
still yields a working bot minus media. The standalone deployment story
`CLAUDE.md` values survives.

There is no retry and no health polling in this design. `GET /health` exists
for operators and compose health checks, not for the bot's own decisions.

## Per-user voice: `Option<i64>` becomes `Option<String>`

`user_settings.tts_speaker_id` holds a Kokoro integer. OpenAI's field is
`voice: String` (`"alloy"`). The column becomes `tts_voice TEXT`, nullable.

`Option` semantics are unchanged and still load-bearing: `None` means "never
customised, use the house default". This is exactly why `CLAUDE.md` made it an
`Option` rather than a plain integer — speaker 0 is a real voice — and `"0"` as
a string has the same property.

Consequences:

- `engine::clamp_speaker_id` moves into the inference crate. Only that crate
  knows how many voices the loaded model has.
- `MAX_TTS_SPEAKER_ID` validation in `api/settings.rs` is removed. The bot
  cannot validate a voice name for a provider it does not know. An unknown
  voice fails at synthesis time with the provider's own error.
- `TTS_SPEAKER_ID` becomes `TTS_VOICE`.

Touch points: `config.rs`, `api/settings.rs` and four of its tests,
`db/schema.rs`, `modules/settings` (`!settings voice <name|none>`),
`SettingsModal.tsx`, `core/settings.ts`, and regenerated `openapi.json` +
`api.ts`.

Per `CLAUDE.md`, no compatibility layer is added — the integer path is removed,
not kept alongside.

## Dependencies removed from `backend/Cargo.toml`

`image` · `ort` · `ort-sys` · `paddle-ocr-rs` · `sherpa-onnx` · `opus` ·
`opus-decoder` · `symphonia`

`ogg` stays, for reading a duration from a granule position when a provider
sends no metadata header. It is pure Rust and needs no native toolchain.

`reqwest` gains the `multipart` feature for `/audio/transcriptions`.

Also removed: `cmake` from the Docker build stage, and
`libsherpa-onnx-c-api.so` plus `libonnxruntime.so` from the runtime stage.

`ort` and `ort-sys` are exact-pinned (`=2.0.0-rc.10`) but never referenced
directly; they exist to force one onnxruntime version across `paddle-ocr-rs`
and `sherpa-onnx`. Those pins move with the models.

## Phasing

Each phase ends with a green `cargo test` and a shippable tree.

0. **Scaffold.** Add the `inference` workspace member. Axum server with
   `GET /health` reporting all three as absent. No models. No backend change.
1. **OCR.** `paddle.rs` moves; `POST /ocr` route; `services/ocr` client;
   `api/ocr.rs` and `modules/ocr/mod.rs` call it. Drop `image`,
   `paddle-ocr-rs`.
2. **STT.** `sherpa.rs` and `audio.rs` move; `/audio/transcriptions` route;
   `services/stt` client; `api/voice.rs` and `modules/voice/mod.rs` call it.
   Drop `sherpa-onnx`, `symphonia`, `opus-decoder`. Acceptance includes
   pointing `STT_BASE_URL` at a third-party provider and confirming a
   transcription.
3. **TTS.** `engine.rs`, `opus.rs`, `text.rs` and `compute_waveform` move;
   `/audio/speech` route with the two extension headers; `services/tts` client
   with the OpenAI dialect; the `tts_voice` migration and its frontend changes;
   `tts_spike.rs` example follows. Drop `opus`, `ort`, `ort-sys`. Acceptance
   includes OpenAI as the provider and confirming a voice bubble still renders.
3a. **ElevenLabs dialect.** `TTS_DIALECT` and the ElevenLabs request shaping,
   as a separate increment on top of a working OpenAI path.
4. **`services/llm`.** Extract `modules/ai/client.rs`. Pure move, no behaviour
   change.
5. **Delivery.** Compose file, CI split, Dockerfile, and documentation.

Phase 4 is independent of 1–3 and may land first if the boundary is wanted
early.

## Testing

- Moved code carries its own `#[cfg(test)]` modules into the inference crate
  (`paddle.rs`, `audio.rs`, `sherpa.rs`, `engine.rs`, `opus.rs`, `text.rs`).
- Each inference route gets a handler test.
- Each `services/*` client is tested against a mock HTTP server, including the
  TTS path where extension headers are **absent**, asserting the Ogg granule
  fallback and `waveform: None`.
- A degradation test per feature: base URL unset, and service unreachable.
- A dialect test asserting that `openai` and `elevenlabs` produce the expected
  auth header, URL and body for the same input, against a mock server.
- The existing suite (377 tests) must stay green. The total shifts as tests
  move between crates; `cargo test --workspace` is the new full run, while bare
  `cargo test` continues to cover the backend.

## CI

The backend job stops downloading the sherpa archive and stops needing
`cmake`. A new `inference` job takes both on. The flake does not disappear —
it stops sitting on the critical path for bot and frontend builds. The
`backend-postgres` job is unaffected.

## Deployment

`docker-compose.yml` gains an `inference` service on an internal network. It is
not published to the host; the bot reaches it by service name. No
authentication is configured initially.

**This is a real gap the moment the service moves to another machine.** A
cross-host deployment must add an API key (the clients already send
`Authorization: Bearer`, so the client side is ready) and TLS. Recorded here so
it is a decision rather than an oversight.

## Known unfixed risk

`ort` and `sherpa-onnx` each want their own onnxruntime, which is why the
exact-version pins exist. Putting all three models in one `inference` crate
keeps that coexistence intact — relocated, not resolved.

If it bites, the escape hatch is splitting `inference` into two containers,
which is cheap **because** the HTTP boundary exists. That is an argument for
the boundary, not a claim that this change fixes the collision.

## Documentation to update

`CLAUDE.md` (the voice-replies section substantially, plus the working-directory
gotchas and a new section on the two crates), `AGENTS.md` (module checklist),
`README.md`, `Dockerfile`, `docker-compose.yml`, and `.env.example`.
