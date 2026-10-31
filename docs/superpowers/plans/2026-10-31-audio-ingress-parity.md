# Omnichannel Audio Ingress Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A linked user's voice note on Telegram or Discord (DM) is transcribed and handled exactly like a Matrix voice note: an AI turn with the user's tools, or a saved note when the caption starts `/note`.

**Architecture:** `process_audio` and `capture_voice_note` are already channel-neutral, so no new transcription path is written. A pure `domain/channels/audio.rs` validates the payload; `interface/channels/audio_ingress.rs` owns the shared flow (resolve identity → rate limit → note or AI turn) and a per-account rate limiter; the STT client and voice config reach it through a `VoiceRuntime` attached to `Inbound`. Telegram and Discord only detect the audio, download it within bounds and call the shared flow.

**Tech Stack:** Rust, `teloxide-core` 0.13, `serenity` 0.12.5, existing `SttClient`.

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` (Blueprint 3)

## Global Constraints

- Max audio size 10 MiB; max duration 120 seconds.
- Allowed MIME types: `audio/ogg`, `audio/opus`, `audio/mpeg`, `audio/mp3`, `audio/wav`, `audio/m4a`.
- 15-second timeout on the media download; STT's own 120 s client timeout stays as is.
- 5 transcriptions per minute per account.
- STT unconfigured or failing: an informative reply, never a crashed worker.
- No schema change, no `OWNED_TABLES` change, no OpenAPI change. Audio is transient; only the resulting note or tool effects persist.
- No SQL in `interface/`; no file over 500 production lines (`discord/inbound.rs` and `discord/worker.rs` are near it, so new Discord code goes in `discord/audio.rs`).

## Rulings (recorded in HANDOFF)

- **Parity with Matrix, not the spec's transcript routing.** The spec routes by a `/note`/`/voice` prefix *in the transcript* and by command prefixes. Matrix uses the *caption* prefix `/note` or `!note` for notes and otherwise runs an AI turn that has every tool. Telegram and Discord do the same, so the three channels cannot drift. Cost if wrong: a user who wants a note says "/note" in the caption instead of aloud.
- **Direct chats only.** Discord already ignores guild messages; Telegram voice is handled in private chats only, because a voice note carries no `@mention`. Cost if wrong: group voice notes are silently ignored.
- **Matrix keeps its own handler.** Its flow is entangled with stage indicators; rewriting it is a refactor with no user value here. Cost if wrong: a small, known duplication of the note-vs-turn branch.
- **No frontend change.** `NoteCard` already shows `source_channel`, and voice notes carry the `voice` tag. The spec's STT status badge in Settings is dropped (YAGNI; the admin vault already shows STT config).
- **`SttUnavailable` / `TranscriptionFailed` are not domain errors.** `process_audio` already turns every `SttServiceError` into a reply, so only the payload errors exist in the domain.

## Review Focus

- Caption `/note` on Telegram/Discord saves a note with `source_channel` `telegram`/`discord` (today the helper hardcodes `matrix`).
- A 10 MiB + 1 byte attachment is refused from metadata, before any download.
- Unlinked sender gets `UNLINKED_REPLY`; STT is never called.
- The 6th clip inside a minute is refused without calling STT; another account is unaffected.
- `audio/ogg; codecs=opus` is accepted; `audio/flac`, a missing MIME with a `.txt` name, and a 121 s clip are refused.
- STT not configured: friendly reply, no panic.

---

### Task 1: Audio payload domain type

**Files:**
- Create: `backend/src/domain/channels/audio.rs`
- Modify: `backend/src/domain/channels/mod.rs` (declare module, re-export)

**Interfaces:**
- Produces: `pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;` `pub const MAX_AUDIO_SECS: u32 = 120;` `pub enum AudioMime { Ogg, Opus, Mpeg, Wav, M4a }` with `fn as_mime(self) -> &'static str`; `pub enum AudioIngressError { Empty, PayloadTooLarge, TooLong, UnsupportedFormat }` (`thiserror`, with a user-facing `fn reply_text(&self) -> &'static str`); `pub struct AudioPayload` (private fields) with `AudioPayload::new(bytes: Vec<u8>, mime: Option<&str>, filename: Option<&str>, duration_secs: Option<u32>) -> Result<Self, AudioIngressError>` and `bytes()`, `mime()`, `duration_secs()`; `pub fn check_declared_size(bytes: u64) -> Result<(), AudioIngressError>` for the pre-download check.

- [ ] **Step 1:** Tests in the same file: `accepts_ogg_with_codec_parameter` (`"audio/ogg; codecs=opus"`), `maps_aliases` (`audio/x-m4a`, `audio/mp4`, `audio/x-wav`, `audio/mp3` → their variants), `falls_back_to_the_filename_extension_when_mime_is_missing_or_generic` (`None`/`application/octet-stream` + `voice.OGG`), `rejects_flac_and_unknown_extensions` (`UnsupportedFormat`), `rejects_empty`, `rejects_one_byte_over_ten_mib` (`PayloadTooLarge`) while exactly 10 MiB passes, `rejects_121_seconds_and_accepts_120` (`TooLong`), `check_declared_size_matches_the_limit`.
- [ ] **Step 2:** Run `cargo test -p matrix-bot domain::channels::audio` — expect FAIL (module missing).
- [ ] **Step 3:** Implement. Mime is lower-cased and trimmed at `;` before matching; extension fallback applies only when the MIME is absent or `application/octet-stream`. Only `std` + `thiserror`.
- [ ] **Step 4:** Run the same command — expect PASS.
- [ ] **Step 5:** `git add backend/src/domain/channels && git commit -m "feat: audio payload domain type for channel voice ingress"`

### Task 2: Per-account rate limiter

**Files:**
- Create: `backend/src/interface/channels/audio_ingress.rs` (limiter only for now)
- Modify: `backend/src/interface/channels/mod.rs`

**Interfaces:**
- Produces: `pub struct AudioRateLimit` (`Mutex<HashMap<AccountId, VecDeque<Instant>>>`) with `AudioRateLimit::new(max: usize, window: Duration)`, `fn allow(&self, owner: &AccountId, now: Instant) -> bool`; `pub const AUDIO_PER_MINUTE: usize = 5;`.

- [ ] **Step 1:** Look for an existing keyed sliding-window limiter (`grep -rn "RateLimit" backend/src`); reuse it if it takes an arbitrary key, else proceed.
- [ ] **Step 2:** Tests: `allows_five_then_refuses_the_sixth`, `a_second_account_is_independent`, `slots_free_after_the_window` (drive `now` forward), `empty_accounts_are_pruned` (map length returns to 0 after the window, so memory stays bounded).
- [ ] **Step 3:** Run `cargo test -p matrix-bot audio_ingress` — FAIL, then implement (prune expired entries on every call; remove the key when its deque empties; poisoned lock allows the call), then PASS.
- [ ] **Step 4:** Commit `feat: per-account audio transcription rate limit`.

### Task 3: Channel-aware voice note capture

**Files:**
- Modify: `backend/src/interface/chat/knowledge.rs:397-470` (`capture_voice_note`), `backend/src/interface/channels/matrix/attachments.rs:242`

**Interfaces:**
- Produces: `capture_voice_note(services, source_channel: &str, source_ref: &str, owner_id, caption, media_bytes, mimetype, voice_config, stt, progress) -> String` — `room_id` renamed `source_ref`, new `source_channel` before it; stored via `.source(source_channel, Some(source_ref.to_string()))`.

- [ ] **Step 1:** Add test `voice_note_records_the_calling_channel` beside the existing note tests (knowledge.rs:~400): stub STT server as in `stt/mod.rs` tests, call with `"telegram"`, assert `note.source_channel == "telegram"` and tag `voice`.
- [ ] **Step 2:** Run it — FAIL (signature). Change the signature, pass `"matrix"` from `attachments.rs`, run again — PASS; run the existing matrix attachment tests.
- [ ] **Step 3:** Commit `refactor: voice note capture records its source channel`.

### Task 4: Shared audio flow on `Inbound`

**Files:**
- Modify: `backend/src/interface/channels/inbound.rs` (add `voice: Option<Arc<VoiceRuntime>>`, `with_voice`, `voice()`), `backend/src/interface/channels/audio_ingress.rs`

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: `pub struct VoiceRuntime { pub config: VoiceConfig, pub stt: SttClient, pub ai: AiEnvConfig, pub limiter: AudioRateLimit }`; `Inbound::with_voice(self, VoiceRuntime) -> Self`; `pub async fn ingest_audio(inbound: &Inbound, sender: &PlatformIdentity, conversation: &ConversationAddress, payload: AudioPayload, caption: Option<&str>, progress: Option<&dyn ProgressSink>) -> Reply`.
- `ingest_audio` order: resolve sender (`Unlinked` → `Reply::html(UNLINKED_REPLY)`, error → `Reply::html` generic notice) → `limiter.allow` (else a "too many voice notes, try again in a minute" reply) → caption starting `/note` or `!note` → `capture_voice_note(.., conversation.channel name, conversation.to_storage(), ..)` else `process_audio(.., conversation.to_storage(), ..)`. No `voice` runtime → the same "voice unavailable" reply `process_audio` gives for `NotConfigured`.

- [ ] **Step 1:** Integration tests with `test_pool()` and a local stub STT server returning `{"text":"buy milk"}`: `unlinked_sender_gets_the_link_prompt_and_stt_is_not_called` (stub counts requests), `note_caption_creates_a_note_for_the_owner`, `sixth_clip_is_refused_without_calling_stt`, `unconfigured_stt_replies_instead_of_panicking`. The plain-voice AI path is covered by `process_audio`'s own tests; assert only that the caption-less call reaches STT.
- [ ] **Step 2:** Run `cargo test -p matrix-bot audio_ingress` — FAIL; implement; PASS.
- [ ] **Step 3:** Commit `feat: shared channel audio ingress flow`.

### Task 5: Telegram voice and audio

**Files:**
- Create: `contracts/channels/telegram_private_voice.json`, `backend/src/interface/channels/telegram/audio.rs`
- Modify: `telegram/inbound.rs` (no change beyond a `pub(super)` helper if `identity` is needed), `telegram/worker.rs` (`Prepared::Audio`, `run_audio`), `telegram/mod.rs` (declare module)

**Interfaces:**
- Produces: `pub struct TelegramAudio { pub chat_id: i64, pub message_id: i32, pub sender: PlatformIdentity, pub file_id: String, pub file_size: u32, pub mime: Option<String>, pub filename: Option<String>, pub duration_secs: u32, pub caption: Option<String> }`; `pub fn audio_of(update: &Update) -> Option<TelegramAudio>` — private chats only, non-bot senders, `voice()` (default mime `audio/ogg`) or `audio()`.

- [ ] **Step 1:** Fixture a private voice update (`voice.file_id`, `file_size`, `duration`, `mime_type: "audio/ogg"`) and tests: `a_private_voice_note_is_extracted`, `a_group_voice_note_is_ignored`, `a_bot_sender_is_ignored`, `a_text_message_yields_none`, `caption_is_carried`, plus `prepare_dispatches_audio` in `worker.rs` (and a duplicate `update_id` is dropped).
- [ ] **Step 2:** Run `cargo test -p matrix-bot channels::telegram` — FAIL; implement `audio_of` and route it in `prepare` before `normalize`.
- [ ] **Step 3:** Implement `run_audio(ctx, audio)`: refuse from `file_size` via `check_declared_size` → `bot.get_file` + `bot.download_file` into a `Vec` inside one `tokio::time::timeout(15s)` → `AudioPayload::new` → `ingest_audio` with `TelegramProgress` → `post`. Every failure replies with `AudioIngressError::reply_text` or a download notice; none panics.
- [ ] **Step 4:** Run the telegram tests and `cargo check -p matrix-bot --all-targets` — PASS. Commit `feat: telegram voice note ingress`.

### Task 6: Discord voice and audio

**Files:**
- Create: `backend/src/interface/channels/discord/audio.rs`
- Modify: `discord/gateway.rs:155-179`, `discord/worker.rs` (`Work::DirectAudio`, dispatch to `audio::run`), `discord/mod.rs`

**Interfaces:**
- Produces: `pub fn is_audio_attachment(a: &Attachment) -> bool` (MIME `audio/*` or extension `.ogg .opus .mp3 .wav .m4a`); `pub async fn run(inbound: &Inbound, http: &Arc<Http>, work: DirectAudio)`.

- [ ] **Step 1:** Tests with `attachment_fixture` (copy the helper's JSON shape from `inbound.rs:739`): `detects_audio_by_mime_and_extension`, `images_and_pdfs_are_not_audio`, `audio_wins_over_image_when_both_attached` (gateway picks audio first, so a voice message is never treated as a receipt).
- [ ] **Step 2:** Run — FAIL; implement. In `gateway.rs` check audio before images. `run`: precheck `attachment.size`, 15 s download, `AudioPayload::new(bytes, content_type, filename, attachment.duration_secs.map(|d| d.ceil() as u32))` (if serenity 0.12.5 lacks `duration_secs`, pass `None`), `ingest_audio` with `TypingProgress`, `reply_dm`.
- [ ] **Step 3:** Run discord tests; commit `feat: discord voice message ingress`.

### Task 7: Wiring, docs, gates

**Files:**
- Modify: `backend/src/app.rs:261-267`, `docs/architecture.md` (one paragraph under channels), `docs/superpowers/HANDOFF.md` (status row, the rulings above)

- [ ] **Step 1:** In `app.rs` build `VoiceRuntime` from `config.voice_enabled`, `voice_language`, `stt_base_url`, `stt_api_key`, `stt_model`, `config.ai.clone()` and `AudioRateLimit::new(AUDIO_PER_MINUTE, Duration::from_secs(60))`; chain `.with_voice(..)` on `Inbound::new`.
- [ ] **Step 2:** Run the §8 gates: `cargo fmt --all -- --check`, `cargo check -p matrix-bot --all-targets`, `cargo clippy -p matrix-bot --all-targets -- -D warnings`, `cargo test -p matrix-bot --all-targets` (needs `TEST_DATABASE_URL`; see the test-Postgres memory), `git diff --check`. `cargo sqlx prepare --check` and `generate-openapi` need no run (no queries or DTOs changed); confirm with `git status`.
- [ ] **Step 3:** Update docs and HANDOFF; commit `docs: record audio ingress parity`.
