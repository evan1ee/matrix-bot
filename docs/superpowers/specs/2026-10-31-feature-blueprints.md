# Top 10 High-Leverage Feature Blueprints

**Document ID:** `SPEC-2026-10-31-FEATURE-BLUEPRINTS`  
**Status:** Approved Architectural Specification  
**Date:** 2026-10-31  
**Authors:** Multi-Agent Core Engineering Group  
**Target System:** `matrix-bot` (Rust / Axum / SQLx / React 19 SPA)  

---

## Executive Summary

Following the full completion of the PostgreSQL foundation cutover, omnichannel channel adapters (Matrix, Discord, Telegram, Email), dynamic runtime configuration, and encrypted credential vault, this specification defines the **Top 10 High-Leverage Feature Blueprints** for `matrix-bot`.

Each blueprint is grounded in audited codebase facts, citing precise file paths, database schemas, repository functions, application services, HTTP/SSE interfaces, and React frontend components. Every blueprint adheres strictly to the modular monolith architecture, synchronous pure domain invariants, transaction-outbox consistency, and owner isolation defined in [`AGENTS.md`](../../../AGENTS.md).

Every blueprint explicitly details all five system layers:
1. **Domain Layer:** Invariants, pure types, value objects, domain errors (pure, synchronous, zero transports).
2. **Database Layer:** Migrations, schema DDL, indexes, `OWNED_TABLES` registration, foreign key cascade actions.
3. **Application Layer:** `<F>Service`, transaction boundaries, transactional outbox events (`SystemEvent`), event owner scoping.
4. **Interface Layer:** HTTP REST/SSE endpoints, DTO contracts, chat commands (`BotModule`), background jobs, channel adapters.
5. **Frontend Layer:** React 19 components, module integration, state management, React Query hooks, keyboard accessibility, monochrome styling ([`Design.md`](../../../Design.md)).

---

## Blueprint Index

1. [Blueprint 1: Web AI Assistant Phase 2 & 3 (Threaded Conversations, Auto-Titles, Dynamic Model Badges, and Non-Destructive New Chat)](#blueprint-1-web-ai-assistant-phase-2--3)
2. [Blueprint 2: Universal Calendar Feed (RFC 5545 iCalendar / Webcal Sync with Reminder Source Deduplication)](#blueprint-2-universal-calendar-feed-rfc-5545-icalendar--webcal)
3. [Blueprint 3: Omnichannel Audio Ingress Parity (Telegram & Discord Voice STT via Inference Service)](#blueprint-3-omnichannel-audio-ingress-parity-telegram--discord-stt)
4. [Blueprint 4: Inbound Email Receipt & Invoice Ingestion (MIME Attachment Extraction & OCR Pipeline)](#blueprint-4-inbound-email-receipt--invoice-ingestion)
5. [Blueprint 5: End-to-End Scanned Receipt Preservation & Lightbox Viewer (Linking `receipt_storage_key` to Transactions)](#blueprint-5-scanned-receipt-preservation--lightbox-viewer)
6. [Blueprint 6: Scheduled Morning Briefing Podcast (Kokoro TTS Compilation + Web Dashboard Object Storage Playback)](#blueprint-6-scheduled-morning-briefing-podcast)
7. [Blueprint 7: Second Brain Bidirectional Wiki-Links (`[[Note Title]]`) & Visual Knowledge Graph](#blueprint-7-second-brain-bidirectional-wiki-links--knowledge-graph)
8. [Blueprint 8: Unified Multi-Domain Knowledge Search & Deletion Cleanup (Plumbing Journal & Footprints, Fixing Embedding Deletion Leaks)](#blueprint-8-unified-multi-domain-knowledge-search--deletion-cleanup)
9. [Blueprint 9: Global Command Palette (`Cmd+K` / `Ctrl+K`) Across All 20 Web Modules](#blueprint-9-global-command-palette-cmdk--ctrlk)
10. [Blueprint 10: Autonomous Scheduled Agent Task Controls (Pause, Resume, Immediate Trigger, & Delete)](#blueprint-10-autonomous-scheduled-agent-task-controls)

---

## Blueprint 1: Web AI Assistant Phase 2 & 3

### 1. Problem Statement & User Value
Phase 1 of the Web AI Assistant delivered streaming text deltas, interactive tool steps, and safe-point turn cancellation via `POST /api/v1/ai/chat/stream`. However, the web UI remains single-conversation:
1. Clicking **New Chat** invokes `DELETE /ai/conversations/:id` ([`AiAssistantView.tsx:156-165`](../../../frontend/src/modules/ai/AiAssistantView.tsx#L156-L165)), permanently destroying the user's conversation history instead of starting a new thread.
2. The UI lacks a multi-thread conversation switcher drawer or dropdown, arbitrarily picking `conversations[0].id` ([`AiAssistantView.tsx:68-70`](../../../frontend/src/modules/ai/AiAssistantView.tsx#L68-L70)).
3. Conversations have no titles in the database ([`0001_baseline.sql:187-196`](../../../backend/migrations/0001_baseline.sql#L187-L196)), defaulting to timestamps or blank titles.
4. The header model badge hardcodes `'gpt-4o-mini'` as a fallback ([`AiAssistantView.tsx:183`](../../../frontend/src/modules/ai/AiAssistantView.tsx#L180-L185)) rather than dynamically reflecting the instance-wide default model from `ResolvedAiConfig`.
5. Phase 3 interactions (regenerating responses, editing earlier user prompts to branch turns, and copying markdown responses) are unavailable.

### 2. Codebase Evidence & Grounding
- **Database Schema:** `backend/migrations/0001_baseline.sql` lines 187–196:
  ```sql
  CREATE TABLE IF NOT EXISTS ai_conversations (
      id TEXT PRIMARY KEY,
      owner_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
      room_id TEXT NOT NULL,
      created_at TEXT NOT NULL DEFAULT (NOW()::text),
      updated_at TEXT NOT NULL DEFAULT (NOW()::text)
  );
  ```
  `owner_id` was migrated to `UUID` in `backend/migrations/0002_account_uuid.sql`. No `title` column exists.
- **Backend Handlers:** `backend/src/interface/http/ai.rs`:
  - `list_conversations` (lines 124–133) calls `llm_repo::list_conversations(&pool, &user.account_id, 20)`.
  - `delete_conversation` (lines 168–184) calls `services.llm.delete_conversation`.
- **Frontend View:** `frontend/src/modules/ai/AiAssistantView.tsx`:
  - Line 183: `{config?.model_name || 'gpt-4o-mini'}`.
  - Lines 156–165 & 201–209: `resetMutation` permanently deletes the conversation.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │                    React Web SPA                       │
 │  ConversationSwitcher ──► Active Thread State (Unset)  │
 │  Dynamic Model Badge  ◄── GET /account/ai-config       │
 └───────────────────────────┬────────────────────────────┘
                             │ POST /ai/chat/stream
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │            Axum HTTP Interface (ai.rs / ai_stream)     │
 │  POST /ai/conversations (Mint new conversation)        │
 │  PATCH /ai/conversations/:id/title (Rename)            │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │                 Application (LlmService)               │
 │  Enqueues ai.generate_title background job on turn 1  │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │                PostgreSQL 18 Storage                   │
 │  ai_conversations (id, owner_id, room_id, title)       │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/llm/`)
- Add `ConversationTitle`: Validated string, trimmed, 1–100 UTF-8 characters, rejecting raw control characters.
- Add `ConversationFilter`: Paging parameters (`limit: usize`, `before: Option<DateTime<Utc>>`).
- Pure title generator heuristic: Truncates user prompt at word boundary up to 50 characters when LLM auto-summary is unconfigured or disabled.
- Add `TurnBranch`: Pure representation of message branching (`parent_message_id: Option<Uuid>`).

#### Database Layer
Migration `backend/migrations/0015_ai_conversation_titles.sql`:
```sql
ALTER TABLE ai_conversations ADD COLUMN title TEXT;
CREATE INDEX idx_ai_conversations_owner_updated ON ai_conversations(owner_id, updated_at DESC);
```
Note: `ai_conversations` is already registered in `OWNED_TABLES` in `backend/src/domain/accounts/mod.rs`.

#### Application Layer (`backend/src/application/llm/`)
- `LlmService::create_conversation(owner_id: &AccountId, room_id: &str) -> Result<AiConversation, AppError>`: Explicitly mints an empty conversation with `None` title.
- `LlmService::update_conversation_title(owner_id: &AccountId, conversation_id: &str, title: ConversationTitle) -> Result<(), AppError>`: Updates row in database and publishes `SystemEvent::AiConversationTitleUpdated`.
- In `LlmService::run_turn`: If the turn completes successfully and `conversation.title` is `NULL` (first turn), enqueue a background job `NewJob::ai_generate_title(owner, conversation_id, prompt, first_reply)` to summarize into a concise 4–6 word title without blocking the HTTP response stream.

#### Interface Layer (`backend/src/interface/http/ai.rs`)
- `POST /api/v1/ai/conversations`: Mint a new conversation ID. Returns `201 Created` with `{ "id": "...", "created_at": "..." }`.
- `PATCH /api/v1/ai/conversations/{id}/title`: Update conversation title with JSON `{ "title": "..." }`.
- `GET /api/v1/account/ai-config`: Return `effective_model_name` resolved from `ResolvedAiConfig` alongside `model_name`.
- `POST /api/v1/ai/chat/stream`: Accept optional `parent_message_id: Option<Uuid>` for branching turns.
- Outbox event publication: `SystemEvent::AiConversationTitleUpdated` broadcasts over SSE so open tabs update conversation titles in real time.

#### Frontend Layer (`frontend/src/modules/ai/`)
- **Conversation Drawer / Switcher:** Sidebar panel or header popover listing user threads with titles and relative timestamps (`['ai-conversations']`).
- **Non-Destructive New Chat:** "New Chat" button sets `activeConversationId = null` and clears message state, leaving existing threads intact in `ai_conversations`.
- **Dynamic Model Badge:** Renders `config?.effective_model_name || config?.model_name || 'Configured Model'`.
- **Phase 3 Controls:**
  - `MessageItem.tsx`: Add "Copy Markdown" button (copy to clipboard with checkmark feedback) and "Edit & Resend" button on user messages (populates composer with message content, truncating future messages upon submission).

### 4. Invariants & Safety Constraints
- **Owner Isolation:** Cross-account access to `/ai/conversations/:id` strictly returns `404 Not Found`.
- **Prompt Cap:** Prompt length checked at boundary (4,000 characters maximum).
- **Concurrency Bound:** Maximum 2 concurrent streaming turns per account.
- **Title Bounds:** Conversation titles bounded at 100 characters; single-line only.

### 5. Verification & Test Strategy
- **Unit Tests:** Title sanitization, UTF-8 truncation at grapheme boundaries, and word-boundary trimming.
- **Repository Tests:** Verify `ai_conversations.title` persistence, update, and owner isolation in `repositories/llm.rs`.
- **API Tests:** `PATCH /ai/conversations/{id}/title` and non-destructive "New Chat" flow.
- **Playwright E2E:** Create thread, send prompt, verify title update via SSE event, start new chat, and verify switching back to previous thread restores history.

---

## Blueprint 2: Universal Calendar Feed (RFC 5545 iCalendar / Webcal)

### 1. Problem Statement & User Value
Users manage commitments across three decoupled temporal features: Todos (`due_at`), Recurring Bills (`next_due_at`), and Reminders (`trigger_at`). Currently, these events are only accessible inside the matrix-bot UI or chat channels. Users cannot subscribe to their unified agenda from system calendar clients (Apple Calendar, Google Calendar, Microsoft Outlook, Thunderbird). Furthermore, reminders spawned by todos or bills (`reminders.source_kind`) must be deduplicated to prevent calendar clutter.

### 2. Codebase Evidence & Grounding
- **Entities & Tables:**
  - `todos` table: `id`, `owner_id`, `task`, `due_at TIMESTAMPTZ`, `status` ([`domain/todos/mod.rs`](../../../backend/src/domain/todos/mod.rs)).
  - `recurring_bills` table: `id`, `owner_id`, `name`, `amount_minor`, `currency`, `frequency`, `next_due_at TEXT` ([`domain/recurring_bills/mod.rs`](../../../backend/src/domain/recurring_bills/mod.rs)).
  - `reminders` table: `id`, `owner_id`, `message`, `trigger_at TIMESTAMPTZ`, `source_kind TEXT`, `source_id BIGINT`, `status` ([`domain/reminders/mod.rs:110-139`](../../../backend/src/domain/reminders/mod.rs#L110-L139)).
- **Existing Source Linking:**
  - `Reminder::reaction_target` checks `source_kind == Some(ReminderSource::Todo)` ([`domain/reminders/mod.rs:134`](../../../backend/src/domain/reminders/mod.rs#L134-L138)).
  - `repositories/reminder_sources.rs` provides `source_kind` and `source_id` queries.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │   External Calendar Client (Apple/Google Calendar)     │
 └───────────────────────────┬────────────────────────────┘
                             │ GET /api/v1/calendar/feed.ics?token=...
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │           Axum Handler (interface/http/calendar.rs)    │
 │  Validates SHA-256 token hash; rejects revoked tokens  │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │               CalendarService (application)            │
 │  Fetches:                                              │
 │  1. Active todos with due_at                           │
 │  2. Active recurring bills with next_due_at            │
 │  3. Pending reminders                                  │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │             Pure RFC 5545 Engine (domain/calendar)     │
 │  Deduplicates: Suppresses reminders with source_id     │
 │  Generates VCALENDAR with VEVENT, VTODO, & VALARM       │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/calendar/`)
- Pure RFC 5545 generator formatting UTC timestamps (`YYYYMMDDTHHMMSSZ`), folding lines at 75 octets (`\r\n `), and escaping text (`\,`, `\;`, `\\`, `\n`).
- Recurrence rule generation: Recurring bills map `bill.frequency` to `RRULE:FREQ=MONTHLY;INTERVAL=1`, `FREQ=WEEKLY`, or `FREQ=YEARLY`.
- **Deduplication Invariant:**
  ```rust
  pub fn should_emit_reminder(reminder: &Reminder) -> bool {
      // If reminder is tied to a todo or bill, the parent item emits the event/todo;
      // the reminder is suppressed to prevent duplicates.
      reminder.source_kind.is_none() && reminder.source_id.is_none()
  }
  ```
- Stable UID Generation:
  - Todo: `UID:todo-{id}@{domain}`
  - Recurring Bill: `UID:bill-{id}-{due_date}@{domain}`
  - Standalone Reminder: `UID:reminder-{id}@{domain}`

#### Database Layer
Migration `backend/migrations/0016_calendar_feed_tokens.sql`:
```sql
CREATE TABLE account_calendar_tokens (
    account_id UUID PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_accessed_at TIMESTAMPTZ
);
CREATE INDEX idx_calendar_token_hash ON account_calendar_tokens(token_hash);
```
Register `account_calendar_tokens` in `OWNED_TABLES` in `backend/src/domain/accounts/mod.rs`.

#### Application Layer (`backend/src/application/calendar/`)
- `CalendarService::get_ics_feed(token_hash: &str) -> Result<String, AppError>`:
  1. Resolves `account_id` from `token_hash`.
  2. Queries active todos with `due_at IS NOT NULL`.
  3. Queries active recurring bills.
  4. Queries pending reminders where `source_kind IS NULL`.
  5. Assembles `IcsFeed` via pure domain formatter.
  6. Updates `last_accessed_at` asynchronously.
- `CalendarService::rotate_token(account_id: &AccountId) -> Result<String, AppError>`: Generates 32-byte cryptographically secure random token (`rand::thread_rng()`), stores SHA-256 hash, and returns plaintext token once.
- `CalendarService::revoke_token(account_id: &AccountId) -> Result<(), AppError>`: Deletes token row, immediately invalidating external calendar subscriptions.

#### Interface Layer (`backend/src/interface/http/calendar.rs`)
- `GET /api/v1/calendar/feed.ics`: Accepts query parameter `?token=...` or Bearer token. Returns `Content-Type: text/calendar; charset=utf-8` with `Cache-Control: private, no-cache`.
- `POST /api/v1/account/calendar-token`: Generates/rotates subscription URL (requires authenticated session).
- `DELETE /api/v1/account/calendar-token`: Revokes subscription URL.

#### Frontend Layer
- In `frontend/src/components/SettingsModal.tsx` and `Dashboard`: Add "Calendar Feed (iCal)" section showing subscription link with `webcal://` one-click subscribe button, token copy button, and "Regenerate Token" warning dialog.

### 4. Invariants & Safety Constraints
- **Timing Attacks & Token Safety:** Plaintext token is never stored; only SHA-256 hash is persisted. Comparison is constant-time (`subtle::ConstantTimeEq`).
- **Deduplication:** A reminder with `source_kind: Some("todo")` or `source_kind: Some("recurring_bill")` MUST NOT produce a standalone `VEVENT`.
- **DoS Prevention:** Max 500 calendar items emitted per feed. Rate-limited to 60 requests per minute per IP.

### 5. Verification & Test Strategy
- **Unit Tests:** Pure RFC 5545 serialization asserting correct timestamp format, line folding at 75 bytes, RRULE recurrence synthesis, and deduplication logic.
- **Repository Tests:** Token hash lookup and owner-scoped cascade deletion on account removal.
- **Contract Tests:** Parse output using standard iCalendar parsers to guarantee compliance across Apple Calendar, Google Calendar, and Outlook.

---

## Blueprint 3: Omnichannel Audio Ingress Parity (Telegram & Discord STT)

### 1. Problem Statement & User Value
Matrix users can send voice notes and audio clips, which are transcribed via `matrix-bot-inference` Whisper STT and routed to command execution or voice note capture (`capture_voice_note`). However, Telegram and Discord channel adapters currently drop inbound voice messages:
- Telegram worker ignores `message.voice()` and `message.audio()` ([`telegram/inbound.rs:56-60`](../../../backend/src/interface/channels/telegram/inbound.rs#L56-L60)).
- Discord gateway ignores voice messages and audio attachments (`Work::DirectAttachment` only processes images in [`discord/worker.rs:168-220`](../../../backend/src/interface/channels/discord/worker.rs#L168-L220)).
Users on Telegram and Discord are forced to type out commands rather than speaking on mobile devices.

### 2. Codebase Evidence & Grounding
- **Matrix Audio Reference Implementation:**
  - `backend/src/interface/channels/matrix/attachments.rs` lines 235–257: Parses `Attachment::Audio`, invokes `knowledge::capture_voice_note` or `chat::voice::process_audio`.
  - Lines 305–370: Resolves owner account, downloads media bytes, passes to `SttClient`.
- **Inference STT Client:** `backend/src/infrastructure/inference/stt/mod.rs` (`SttClient::transcribe`).
- **Telegram Ingress:** `backend/src/interface/channels/telegram/inbound.rs`: Lines 56–60 return `None` for messages without text.
- **Discord Ingress:** `backend/src/interface/channels/discord/worker.rs`: Lines 168–220 implement `Work::DirectAttachment` for images; audio MIME types are rejected.

### 3. Architectural Design

```text
 ┌──────────────────────┐        ┌──────────────────────┐
 │ Telegram Voice Note  │        │ Discord Voice Clip   │
 └──────────┬───────────┘        └──────────┬───────────┘
            │                               │
            ▼                               ▼
 ┌──────────────────────────────────────────────────────┐
 │           Channel Inbound Normalization              │
 │  Telegram: Download file path via bot.get_file       │
 │  Discord: Download attachment stream (< 10 MiB)      │
 └──────────────────────────┬───────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────┐
 │       Shared Inbound Audio Pipeline (inbound.rs)     │
 │  1. Resolve PlatformIdentity -> AccountId            │
 │  2. Check SttClient configuration & health           │
 │  3. Transcribe audio bytes via matrix-bot-inference  │
 └──────────────────────────┬───────────────────────────┘
                            │
                            ▼
 ┌──────────────────────────────────────────────────────┐
 │          Unified Inbound Command Dispatcher          │
 │  Prefix: /note -> capture_voice_note                 │
 │  Other:  InboundText -> BotModule execution          │
 └──────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/channels/audio.rs`)
- Add `AudioPayload`: Validated binary buffer, duration bounds (`duration_seconds: Option<u32>`), MIME classification (`AudioMime`).
- Allowed MIME types: `audio/ogg`, `audio/opus`, `audio/mpeg`, `audio/mp3`, `audio/wav`, `audio/m4a`.
- Invariants: Max audio file size = 10 MiB; max duration = 120 seconds.
- Add `AudioIngressError` enum (`PayloadTooLarge`, `UnsupportedFormat`, `TranscriptionFailed`, `SttUnavailable`).

#### Database Layer
- Schema & migration evaluation: No schema changes required. Audio ingress streams are transient buffers processed in-flight. Resulting notes or tasks are stored in existing tables (`notes`, `todos`) via existing repositories, respecting `OWNED_TABLES`.

#### Application Layer (`backend/src/application/`)
- Integrates with `InboundDispatchService` and `KnowledgeService::capture_voice_note`:
  1. If transcribed text is prefixed with `/note` or `/voice`, saves note in write transaction and publishes `SystemEvent::NoteCreated`.
  2. If transcribed text matches registered command prefixes (`todo`, `expense`, etc.), executes command within service transaction and publishes the respective outbox event.
  3. If general text or conversational turn, dispatches to `LlmService::run_turn`.

#### Interface Layer (`backend/src/interface/channels/`)
- **Shared Audio Normalizer (`backend/src/interface/channels/audio_ingress.rs`):**
  ```rust
  pub async fn process_channel_audio(
      services: &AppServices,
      stt: &SttClient,
      owner_id: &AccountId,
      audio_bytes: Vec<u8>,
      mime_type: Option<&str>,
      caption: Option<&str>,
  ) -> Result<String, AudioIngressError>
  ```
- **Telegram Adapter (`backend/src/interface/channels/telegram/`):**
  - In `inbound.rs`: Detect `message.voice()` or `message.audio()`.
  - In `worker.rs`: Download audio bytes via `bot.get_file(file_id)` within a 15-second timeout, resolve identity, transcribe via `SttClient`, and feed the transcribed text into `inbound::dispatch`.
- **Discord Adapter (`backend/src/interface/channels/discord/`):**
  - In `gateway.rs`: Check `attachment.content_type` for `audio/*` (or extension `.ogg`, `.mp3`, `.m4a`).
  - In `worker.rs`: Download bytes (enforcing 10 MiB limit), transcribe via `SttClient`, and route to command pipeline.

#### Frontend Layer
- In Web SPA (`Notes` module, `Todo` module): Transcribed voice notes display a microphone metadata badge (`🎙️ Voice Note`) indicating omnichannel voice ingress origin.
- Settings modal (`Admin` / `Settings`): Display STT inference service connectivity status and Whisper model indicator.

### 4. Invariants & Safety Constraints
- **Payload Limits:** Maximum audio payload size capped at 10 MiB.
- **Timeout Bound:** 15-second timeout on media downloads; 30-second timeout on STT inference requests.
- **Fail-Open Graceful Degradation:** If `SttClient` is unconfigured or inference service returns an error, send an informative user reply (`"🎙️ Could not transcribe voice note: STT service unavailable"`) without crashing the channel worker.
- **Rate Limit:** 5 audio transcriptions per minute per user to protect inference server resources.

### 5. Verification & Test Strategy
- **Unit Tests:** Mock audio payload parsing and format validation (`ogg`, `mp3`, `wav`).
- **Integration Tests:** Wire mock HTTP server for inference STT responses, verifying Telegram and Discord worker dispatch flows.
- **Error Handling Tests:** Verify behavior on network disconnect during audio download and malformed audio formats.

---

## Blueprint 4: Inbound Email Receipt & Invoice Ingestion

### 1. Problem Statement & User Value
Users receive transactional receipts, PDF invoices, and booking confirmations via email (e.g. from airlines, hotels, ride-shares, SaaS subscriptions). Today, inbound email parses plain text and HTML bodies ([`inbound.rs:80-85`](../../../backend/src/interface/channels/email/inbound.rs#L80-L85)), but completely discards attachments in `mail_parser::Message::attachments()`. Users must manually download email receipts and re-upload them in the web console or Matrix chat to extract expenses.

### 2. Codebase Evidence & Grounding
- **Inbound Email Module:** `backend/src/interface/channels/email/inbound.rs`: Uses `mail_parser::{Message, MessageParser}` and `verify_dmarc`. Lines 80–85: `ParsedEmail` contains only `message`, `text`, `subject`.
- **Receipt Job Engine:** `backend/src/interface/jobs/media.rs`:
  - `JobType::MediaProcessReceipt` (lines 175–230): Runs PaddleOCR scan, extracts amount, merchant, category, and calls `services.finance.record_expense(..., Some(storage_key), ...)`.
- **Storage Service:** `backend/src/infrastructure/storage/`: Supports `StorageCategory::Receipts`.
- **Finance Service:** `backend/src/application/finance/mod.rs` (`record_expense`).

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │           Inbound Email (IMAP Polling / TLS)           │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │          DMARC Verification (verify_dmarc)             │
 │  Reject unauthenticated / forged external senders      │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │       MIME Attachment Extraction (mail_parser)         │
 │  Filter: application/pdf, image/png, image/jpeg        │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │        Storage & Asynchronous Job Enqueue              │
 │  1. Put bytes -> StorageCategory::Receipts             │
 │  2. Enqueue JobType::MediaProcessReceipt               │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │         Durable Worker (MediaProcessReceiptHandler)    │
 │  1. Runs PaddleOCR on image/PDF pages                  │
 │  2. Extracts merchant, date, amount_minor, currency    │
 │  3. Calls FinanceService::record_expense               │
 │  4. Replies to sender via SMTP with receipt summary    │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/channels/email/`)
- Add `InboundAttachment`: Filename, MIME type (`content_type`), raw payload bytes.
- Allowed MIME types: `application/pdf`, `image/jpeg`, `image/png`, `image/webp`.
- PDF bounding invariant: Only first 3 pages parsed for OCR to prevent decompression bombs.
- Money parsing invariant: Extracted amounts must be strictly parsed into integer `amount_minor: i64`, `currency: String`, and `currency_exponent: i64` (default 2) per [`AGENTS.md`](../../../AGENTS.md) rules.

#### Database Layer
- Schema analysis: Uses existing `transactions` table with `receipt_storage_key TEXT` from migration `0008_transaction_receipt_storage_key.sql`.
- Idempotency & deduplication: Enforces unique job key `job_key = "email_receipt:{message_id}:{attachment_idx}"` in `jobs` table to guarantee that IMAP re-synchronizations or duplicate fetches never duplicate expenses.

#### Application Layer (`backend/src/application/finance/`)
- `FinanceService::record_expense`:
  Inside write transaction: inserts transaction row with `receipt_storage_key = Some(storage_key)`, publishes `SystemEvent::TransactionCreated`.
- Outbox event published: `SystemEvent::TransactionCreated { owner_id, transaction_id }` broadcast to SSE subscribers.

#### Interface Layer (`backend/src/interface/`)
- **Email Inbound Parser Extension (`backend/src/interface/channels/email/inbound.rs`):**
  Update `ParsedEmail`:
  ```rust
  pub struct InboundAttachment {
      pub filename: String,
      pub content_type: String,
      pub data: Vec<u8>,
  }

  pub struct ParsedEmail {
      pub message: IncomingMessage,
      pub text: InboundText,
      pub subject: Option<String>,
      pub attachments: Vec<InboundAttachment>,
  }
  ```
  Extract attachments from `mail_parser::Message::attachments()`, filtering for allowed MIME types.
- **Inbound Ingestion Pipeline (`backend/src/interface/channels/email/adapter.rs`):**
  1. Verify sender identity via `verify_dmarc` against configured `authserv_id`.
  2. Resolve owner `AccountId`.
  3. Put bytes into `StorageCategory::Receipts` and enqueue `NewJob::media_process_receipt(owner, storage_key, ConversationAddress::Email(from_address))`.
- **Job Handler (`backend/src/interface/jobs/media.rs`):**
  Executes OCR, records expense, and delivers confirmation notification via `EmailAdapter` (SMTP reply via `lettre`) summarizing extracted merchant, amount, category, and transaction ID.

#### Frontend Layer
- In Web SPA Finance module (`TransactionList.tsx`): Email-ingested transactions display an `Email` origin badge and a paperclip icon linking directly to the receipt image preview in `ReceiptLightboxModal`.

### 4. Invariants & Safety Constraints
- **DMARC Security Enforcement:** Receipt ingestion MUST strictly require DMARC pass to prevent spoofed emails from polluting financial records.
- **Attachment Limits:** Max 10 MiB per attachment, max 30 MiB total per message.
- **PDF Page Bounding:** PDF OCR renders only the first 3 pages to prevent decompression bombs.
- **Minor-Unit Currency:** Floating-point numbers are prohibited in persistence and domain arithmetic.

### 5. Verification & Test Strategy
- **Unit Tests:** Test MIME extraction on multi-part emails with embedded PNG, JPEG, and PDF attachments using real RFC 5322 fixtures.
- **Security Tests:** Reject unauthenticated senders failing SPF/DKIM/DMARC.
- **Job Tests:** Verify `MediaProcessReceiptHandler` processes stored email attachments and dispatches SMTP reply upon successful expense creation.

---

## Blueprint 5: Scanned Receipt Preservation & Lightbox Viewer

### 1. Problem Statement & User Value
When receipts are ingested via Matrix or background jobs, the OCR process extracts financial details, saves the image to `StorageCategory::Receipts`, and writes `receipt_storage_key` to the database. However, this feature is disconnected on the web:
1. `TransactionResponse` in `interface/http/finance.rs` ([`lines 80-115`](../../../backend/src/interface/http/finance.rs#L80-L115)) drops `receipt_storage_key`, preventing the web frontend from knowing a receipt exists.
2. Web uploads via `POST /api/v1/ocr/scan` ([`interface/http/ocr.rs:46-85`](../../../backend/src/interface/http/ocr.rs#L46-L85)) pass bytes to OCR but do not store the image in object storage, losing the uploaded file.
3. The web console (`TransactionList.tsx`) lacks a receipt preview/lightbox modal.

### 2. Codebase Evidence & Grounding
- **Database Column:** `transactions.receipt_storage_key TEXT` exists (migration `0008_transaction_receipt_storage_key.sql`).
- **Domain Entity:** `Transaction` in `backend/src/domain/finance/mod.rs:86` has `pub receipt_storage_key: Option<StorageKey>`.
- **Repository Support:** `backend/src/infrastructure/postgres/repositories/finance/transactions.rs` already selects and inserts `receipt_storage_key`.
- **HTTP DTO Gap:** `backend/src/interface/http/finance.rs` lines 80–115: `Transaction` DTO omits `receipt_storage_key`, and `From<domain::Transaction>` omits the field.
- **Web UI Gap:** `frontend/src/modules/finance/types.ts` omits `receipt_storage_key`; `ReceiptDropzone.tsx` drops file after scan.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │                    Web Frontend                        │
 │  1. ReceiptDropzone uploads file                       │
 │  2. TransactionList shows Receipt Thumbnail / Icon     │
 │  3. Clicking opens ReceiptLightboxModal                │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │               Axum REST Interface                      │
 │  POST /api/v1/finance/receipts/upload                  │
 │    -> Stores file, runs OCR, returns storage_key       │
 │  GET /api/v1/finance/transactions/:id/receipt-url      │
 │    -> Generates HMAC-presigned read URL                │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │          StorageService (HMAC Presigning)              │
 │  Presigns GET request with 5-minute expiration         │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/finance/`)
- `StorageKey` domain type in `domain/storage.rs`: Validates alphanumeric/hyphen/slash format, preventing directory traversal (`..`).
- Attachment rules: Exactly one receipt storage key per transaction; only valid image/PDF MIME types; max file size 10 MiB.
- Domain events: `ReceiptAttached { transaction_id, storage_key }`, `ReceiptDetached { transaction_id }`.

#### Database Layer
- Schema verification: `transactions.receipt_storage_key TEXT` and index `idx_transactions_receipt_storage_key` exist from migration `0008_transaction_receipt_storage_key.sql`.
- Ownership: `transactions` is registered in `OWNED_TABLES`. Deleting an account cascades transactions; storage objects are pruned by the storage retention cleaner.

#### Application Layer (`backend/src/application/finance/`)
- `FinanceService::attach_receipt(owner: &AccountId, tx_id: i64, key: StorageKey) -> Result<(), AppError>`: Updates row in write transaction, publishes `SystemEvent::TransactionUpdated`.
- `FinanceService::detach_receipt(owner: &AccountId, tx_id: i64) -> Result<(), AppError>`: Unlinks key, publishes `SystemEvent::TransactionUpdated`.
- `StorageService::presign_get(&storage_key, duration) -> Result<String, AppError>`: Generates HMAC-SHA256 presigned GET URL with 300-second expiration.

#### Interface Layer (`backend/src/interface/http/finance.rs`)
- Add `receipt_storage_key` to `Transaction` DTO:
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
  pub struct Transaction {
      pub id: i64,
      // ...
      pub receipt_storage_key: Option<String>,
  }

  impl From<crate::domain::finance::Transaction> for Transaction {
      fn from(t: crate::domain::finance::Transaction) -> Self {
          Self {
              id: t.id,
              // ...
              receipt_storage_key: t.receipt_storage_key.map(|k| k.to_string()),
          }
      }
  }
  ```
- Endpoints:
  - `POST /api/v1/finance/receipts/upload`: Multipart upload endpoint. Saves image bytes into `StorageCategory::Receipts`, runs OCR extraction, returns `{ receipt_storage_key: String, extracted: OcrExtractedData }`.
  - `GET /api/v1/finance/transactions/{id}/receipt-url`: Presigns HMAC GET URL, returns `{ url: String, expires_in_seconds: 300 }`.
  - `DELETE /api/v1/finance/transactions/{id}/receipt`: Unlinks receipt storage key from transaction.

#### Frontend Layer (`frontend/src/modules/finance/`)
- `ReceiptDropzone.tsx`: Preserves file bytes, uploads to `/api/v1/finance/receipts/upload`, receives `receipt_storage_key`, and binds key into transaction form state.
- `TransactionList.tsx`: Displays receipt icon/thumbnail column for transactions with `receipt_storage_key`.
- `ReceiptLightboxModal.tsx`: Accessible modal with pan/zoom, 90° image rotation, download button, and side-by-side OCR extracted expense details.

### 4. Invariants & Safety Constraints
- **Presigned Expiration:** Presigned URLs expire strictly after 300 seconds.
- **Cross-Account Authorization:** Verified against `user.account_id`; cannot presign receipts belonging to another user.
- **MIME Whitelist:** Only `image/jpeg`, `image/png`, `image/webp`, and `application/pdf` allowed.

### 5. Verification & Test Strategy
- **Repository Tests:** Assert `receipt_storage_key` persists and loads across transactions.
- **API Tests:** Verify `/finance/transactions/:id/receipt-url` returns 404 when no receipt is linked and 403 on cross-owner request.
- **E2E Tests:** Upload receipt image via web UI, create expense, verify paperclip icon appears in table, and open lightbox viewer.

---

## Blueprint 6: Scheduled Morning Briefing Podcast

### 1. Problem Statement & User Value
The system generates a personalized daily morning briefing aggregating weather, calendar commitments, overdue todos, bills due, habit consistency, and spending updates. While text and HTML versions are delivered to chat rooms, users who listen to news during commutes or morning routines cannot consume their briefing as audio. The autonomous agent runtime already includes a multi-modal Kokoro TTS compilation pipeline ([`interface/jobs/agent/audio.rs`](../../../backend/src/interface/jobs/agent/audio.rs)), but it has not been connected to the morning briefing job or the web dashboard.

### 2. Codebase Evidence & Grounding
- **TTS Audio Compilation Engine:** `backend/src/interface/jobs/agent/audio.rs`:
  - `extract_audio_script(&str) -> Option<String>` (lines 18–32).
  - `strip_audio_script(&str) -> String` (lines 37–84).
  - `compile_audio_digest(&AppServices, &AccountId, &str, Option<&str>) -> Option<Attachment>` (lines 89–130).
- **Briefing Job:** `backend/src/interface/jobs/briefing.rs`:
  - `BriefingSendHandler` (lines 20–162): Generates `BriefingPayload`, delivers plain text and HTML to default room, and records sent day in `briefing_sent_days`.
- **Object Storage Audio Category:** `StorageCategory::Audio` exists in `backend/src/infrastructure/storage/`.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │           Scheduled Trigger: briefing.send             │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │         BriefingGenerator (application/briefing)       │
 │  Produces:                                             │
 │  1. HTML & Plain Text Briefing                         │
 │  2. <audio_script>Morning dispatch...</audio_script>   │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │         BriefingSendHandler (interface/jobs)           │
 │  1. compile_audio_digest synthesizes Kokoro TTS audio  │
 │  2. Stores Ogg Opus -> StorageCategory::Audio          │
 │  3. Attaches audio to chat message (Matrix/Discord/TG) │
 │  4. Persists audio_storage_key in briefing_sent_days   │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │            Web Dashboard Podcast Player                │
 │  Renders audio player with waveform, speed controls,   │
 │  and daily episode archives                            │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/briefing/`)
- Add `BriefingAudioScript`: Validated conversational plain-text extracted from `<audio_script>...</audio_script>`.
- Invariants: Script capped at 2,000 characters to prevent excessive inference latency.
- Audio format representation: `AudioFormat::OggOpus`, bitrate 48kbps mono.
- Voice selection: Configured Kokoro voice profile (e.g. `af_heart`, `am_adam`, `bf_emma`).

#### Database Layer
Migration `backend/migrations/0017_briefing_audio_key.sql`:
```sql
ALTER TABLE briefing_sent_days ADD COLUMN audio_storage_key TEXT;
```

#### Application Layer (`backend/src/application/briefing/`)
- `BriefingGenerator` (`backend/src/application/briefing/generator.rs`):
  Generates conversational `<audio_script>` block optimized for spoken delivery alongside HTML/text:
  ```text
  <audio_script>
  Good morning! Here is your daily dispatch for Friday, October 31st.
  Today you have 3 high priority tasks due, starting with database migration rehearsal.
  Your emergency fund runway is strong at 8.4 months. Keep up the 5-day habit streak!
  Have a productive day.
  </audio_script>
  ```
- Synthesis orchestration: If TTS is enabled in config snapshot, compiles audio via `compile_audio_digest(&services, &owner, &script, voice_profile)`, stores Ogg Opus in `StorageCategory::Audio` under `audio/briefings/{owner}/{date}.opus`, and records `audio_storage_key` in `briefing_sent_days`. Publishes `SystemEvent::BriefingGenerated`.

#### Interface Layer (`backend/src/interface/`)
- Job Handler (`backend/src/interface/jobs/briefing.rs`): `BriefingSendHandler` synthesizes audio and attaches `AttachmentKind::Audio` to chat message in Matrix, Discord, or Telegram.
- REST Endpoint: `GET /api/v1/briefing/audio?date=YYYY-MM-DD`: Returns presigned HMAC streaming URL (`{ url, expires_in_seconds: 3600 }`).

#### Frontend Layer (`frontend/src/modules/dashboard/`)
- `BriefingCard.tsx`: Embed HTML5 `<audio>` player with play/pause, scrub bar, playback speed toggles (1x, 1.25x, 1.5x, 2x), and download button.
- `useBriefingAudio(date)` hook managing audio streaming state and audio session playback.

### 4. Invariants & Safety Constraints
- **Fail-Open Delivery:** If Kokoro TTS fails or inference is offline, text/HTML delivery MUST still succeed without throwing a job failure.
- **Audio Length Bound:** Scripts capped at 2,000 characters to prevent excessive inference latency.
- **Storage Lifecycle:** Audio briefings retained for 30 days, then pruned by storage cleanup workers.

### 5. Verification & Test Strategy
- **Unit Tests:** Verify `<audio_script>` tag extraction, whitespace normalization, and tag stripping from plain text.
- **Job Tests:** Verify briefing delivery with mock TTS client returns both audio attachment and persists `audio_storage_key`.
- **E2E Tests:** Dashboard loads briefing audio player and streams Ogg Opus audio.

---

## Blueprint 7: Second Brain Bidirectional Wiki-Links & Knowledge Graph

### 1. Problem Statement & User Value
The Notes module (`domain/knowledge/note.rs`) supports structured notes, idea tags, and semantic pgvector embeddings. However, notes exist in isolation:
1. Users cannot link related thoughts using standard Second Brain syntax (`[[Note Title]]`).
2. There is no automated backlink indexing (discovering which other notes reference the current note).
3. The web console lacks an interactive knowledge graph visualizing connections, clusters, and orphan thoughts across the user's personal knowledge base.

### 2. Codebase Evidence & Grounding
- **Note Domain Entity:** `backend/src/domain/knowledge/note.rs`:
  - `NoteId(pub i64)`, `Note`, `MAX_TITLE_LEN = 200`, `MAX_CONTENT_CHARS = 100_000`.
- **Repository Implementation:** `backend/src/infrastructure/postgres/repositories/knowledge/notes.rs`.
- **Application Service:** `backend/src/application/knowledge/notes.rs`:
  - `create_note`, `update_note`, `delete_note`.
- **Web Frontend:** `frontend/src/modules/knowledge/`: Contains `KnowledgeMainView.tsx`, `NoteEditor.tsx`, `NoteList.tsx`.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │                    Web Note Editor                     │
 │  Types "[[" -> Triggers Title Autocomplete Popup      │
 └───────────────────────────┬────────────────────────────┘
                             │ Save Note (POST / PUT)
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │           Domain Parser (domain/knowledge/links.rs)    │
 │  Extracts [[Title]] and [[Title|Display Text]] matches │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │               NoteService (application)                │
 │  Inside Write Transaction:                             │
 │  1. Updates note row                                   │
 │  2. Resolves target note titles to NoteId              │
 │  3. Replaces rows in knowledge_note_links              │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │               Visual Knowledge Graph                   │
 │  GET /api/v1/notes/graph -> Nodes & Edges              │
 │  D3 / Canvas Force-Directed Interactive Visualizer     │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/knowledge/links.rs`)
Pure regex / string scanner:
- Extracts all occurrences of `\[\[([^\]\|]+)(?:\|([^\]]+))?\]\]`.
- Normalizes target titles (trimmed, lowercased for index matching).
- Caps extracted links at 50 per note to prevent unbounded relational fan-out.
- Filters out self-links and ignores matches inside fenced code blocks or backtick expressions.

#### Database Layer
Migration `backend/migrations/0018_knowledge_note_links.sql`:
```sql
CREATE TABLE knowledge_note_links (
    id BIGSERIAL PRIMARY KEY,
    owner_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    source_note_id BIGINT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
    target_note_id BIGINT REFERENCES notes(id) ON DELETE SET NULL,
    target_title TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_note_links_owner_source ON knowledge_note_links(owner_id, source_note_id);
CREATE INDEX idx_note_links_owner_target ON knowledge_note_links(owner_id, target_note_id);
CREATE INDEX idx_note_links_owner_title ON knowledge_note_links(owner_id, LOWER(target_title));
```
Register `knowledge_note_links` in `OWNED_TABLES` in `backend/src/domain/accounts/mod.rs`.

#### Application Layer (`backend/src/application/knowledge/notes.rs`)
Inside the `create_note` and `update_note` transactions:
1. Parse wiki-links from `content`.
2. Resolve target titles against existing user notes:
   ```sql
   SELECT id, title FROM notes WHERE owner_id = $1 AND LOWER(title) = ANY($2)
   ```
3. Delete previous outgoing links for `source_note_id`.
4. Insert updated records into `knowledge_note_links`.
5. Publish `SystemEvent::NoteUpdated`.

#### Interface Layer (`backend/src/interface/http/knowledge.rs`)
- `GET /api/v1/notes/{id}/links`: Returns `{ outgoing: Vec<NoteLinkDto>, backlinks: Vec<NoteLinkDto> }`.
- `GET /api/v1/notes/graph`: Returns `{ nodes: Vec<GraphNode>, edges: Vec<GraphEdge> }` for all notes owned by user.

#### Frontend Layer (`frontend/src/modules/knowledge/`)
- **Autocomplete Popup:** In `NoteEditor.tsx`, listening for `[[` activates floating dropdown showing existing note titles; selecting one completes the bracket.
- **Link Renderer:** In note reading view, renders `[[Title]]` as an internal route link to that note.
- **Backlinks Sidebar:** Displays "Mentioned In" section showing all backlinks.
- **Graph View:** New tab in Knowledge module rendering a 2D force-directed knowledge graph with node zooming, search filtering, and node click navigation.

### 4. Invariants & Safety Constraints
- **Self-Link Invariant:** A note linking to itself is filtered out.
- **Unresolved Target Safety:** Links to non-existent notes have `target_note_id = NULL` but preserve `target_title`, enabling ghost note discovery ("create note from link").
- **Owner Isolation:** Cross-account link resolution is strictly impossible via `owner_id` constraints.

### 5. Verification & Test Strategy
- **Domain Unit Tests:** Test markdown parser with aliases, edge punctuation, special characters, and code block ignoring (`[[test]]` inside code blocks must not link).
- **Repository Tests:** Verify link replacement inside transactions and cascade deletion when a target note is removed.
- **Playwright E2E:** Create Note A, create Note B with `[[Note A]]`, verify backlink appears in Note A, and verify edge in Graph View.

---

## Blueprint 8: Unified Knowledge Search & Deletion Cleanup

### 1. Problem Statement & User Value
`matrix-bot` includes hybrid vector and keyword search powered by pgvector (`knowledge_embeddings`). While `Note` and `Bookmark` types are partially indexed, two severe architectural flaws exist:
1. **Embedding Deletion Leaks:** When bookmarks (`BookmarkService::delete`) or footprints (`FootprintService::delete`) are deleted, their rows are dropped from the database, but their vector embeddings in `knowledge_embeddings` are never deleted ([`reviews/2026-10-10-tech-debt-audit.md#TD-13`](../reviews/2026-10-10-tech-debt-audit.md)). Stale embeddings leak into search queries permanently, resulting in broken search results.
2. **Missing Ingestion Plumbing:** `KnowledgeItemType` already has variants for `Journal` and `Footprint`, and `get_item_content_for_embedding` already implements extraction queries for them ([`indexing.rs:155-200`](../../../backend/src/infrastructure/postgres/repositories/knowledge/indexing.rs#L155-L200)). However, `LifeService::record_day_log` and `FootprintService::record_footprint` never enqueue embedding jobs.

### 2. Codebase Evidence & Grounding
- **Item Types Enum:** `backend/src/domain/knowledge/search.rs:4-32`:
  `KnowledgeItemType` includes `Note`, `Bookmark`, `Journal`, `Footprint`.
- **Extraction Logic Ready:** `backend/src/infrastructure/postgres/repositories/knowledge/indexing.rs`:
  Lines 155–200 implement extraction for `Journal` (`life_day_logs`) and `Footprint` (`footprints`).
- **Deletion Reference:** `backend/src/application/knowledge/notes.rs:233`:
  `NoteService::delete_note` calls `repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, "note", id).await?;`.
- **The Leaks:**
  - `backend/src/application/bookmarks/mod.rs:77`: `delete` deletes bookmark row, never deletes embeddings.
  - `backend/src/application/footprints/mod.rs:114`: `delete` deletes footprint row, never deletes embeddings.
  - `backend/src/application/life/mod.rs:88`: `delete_day_log` deletes journal row, never deletes embeddings.
- **Versioned Job Keys:** `backend/src/domain/jobs.rs:318-325`:
  `NewJob::knowledge_embed_item_versioned` must be used for updating entities to prevent stale job deduplication.

### 3. Architectural Design

```text
       INGESTION / MUTATION                   DELETION (CLEANUP)
 ┌───────────────────────────────┐     ┌───────────────────────────────┐
 │ LifeService::record_day_log   │     │ BookmarkService::delete       │
 │ FootprintService::record      │     │ FootprintService::delete      │
 └──────────────┬────────────────┘     │ LifeService::delete_day_log   │
                │                      └──────────────┬────────────────┘
                ▼                                     │
 ┌───────────────────────────────┐                    ▼
 │ Enqueue:                      │     ┌───────────────────────────────┐
 │ knowledge_embed_item job      │     │ Inside Write Transaction:     │
 └──────────────┬────────────────┘     │ delete_embeddings_for_item()  │
                │                      └──────────────┬────────────────┘
                ▼                                     │
 ┌───────────────────────────────┐                    ▼
 │ Durable Background Worker     │     ┌───────────────────────────────┐
 │ Embeds & writes pgvector row  │     │ Vector Storage Consistency    │
 └───────────────────────────────┘     │ No Ghost Embeddings           │
                                       └───────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/knowledge/search.rs`)
- `KnowledgeItemType` enum: `Note`, `Bookmark`, `Journal`, `Footprint`.
- Vector dimension invariant: 384 dimensions (`bge-small-en-v1.5`) or configured embedding model size.
- Similarity scoring: Reciprocal Rank Fusion (RRF) combining BM25 keyword rank and vector cosine similarity.
- Search query bounds: 1–200 characters, optional category filter.

#### Database Layer
- `knowledge_embeddings` schema (`0010_knowledge_base.sql`):
  Columns `(owner_id, item_type, item_id, chunk_index, embedding, snippet)`.
  pgvector cosine distance index `idx_knowledge_embeddings_vector`.
- Cleanup query (`backend/src/infrastructure/postgres/repositories/knowledge/search.rs:54-73`):
  ```sql
  DELETE FROM knowledge_embeddings
  WHERE owner_id = $1 AND item_type = $2 AND item_id = $3
  ```

#### Application Layer (`backend/src/application/`)
- **Deletion Cleanup:** In `BookmarkService::delete`, `FootprintService::delete`, and `LifeService::delete_day_log`, execute within the write transaction before commit:
  ```rust
  // BookmarkService
  repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, "bookmark", id).await?;

  // FootprintService
  repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, "footprint", id).await?;

  // LifeService
  repositories::knowledge::delete_embeddings_for_item(&mut *tx, owner, "journal", id).await?;
  ```
- **Ingestion Plumbing:**
  - In `LifeService::record_day_log`:
    ```rust
    let job = NewJob::knowledge_embed_item_versioned(*owner, KnowledgeItemType::Journal, log.id, &log.updated_at);
    repositories::jobs::enqueue(&mut *tx, &job).await?;
    ```
  - In `FootprintService::record_footprint` and `update_footprint`:
    ```rust
    let job = NewJob::knowledge_embed_item_versioned(*owner, KnowledgeItemType::Footprint, footprint.id, &footprint.updated_at.to_rfc3339());
    repositories::jobs::enqueue(&mut *tx, &job).await?;
    ```
- **Search Sanitization:** `SearchService::hybrid_search` checks item existence and silently filters orphaned hits in case of legacy drift.

#### Interface Layer (`backend/src/interface/`)
- REST API: `GET /api/v1/search/hybrid?query=...&types=note,bookmark,journal,footprint&limit=20`.
- BotModule: `!search <query>` chat command returning formatted multi-domain search summary.
- Job handler: `KnowledgeEmbedItemHandler` in `backend/src/interface/jobs/knowledge.rs`.

#### Frontend Layer (`frontend/src/modules/knowledge/`)
- Unified search results display badged icons for item types (`Note`, `Bookmark`, `Journal`, `Footprint`).
- Clicking a result navigates directly to the target item in its respective module.

### 4. Invariants & Safety Constraints
- **Transactional Atomicity:** Embedding deletions MUST occur inside the same transaction as the parent entity row deletion.
- **Fail-Safe Search Results:** If an embedding points to a deleted entity due to legacy drift, `SearchService::hybrid_search` filters out orphaned records on read.
- **Versioned Job Keys:** Using `knowledge_embed_item_versioned` ensures edited journal entries or footprints bypass deduplication against earlier versions.

### 5. Verification & Test Strategy
- **Repository Tests:** Create bookmark/footprint, insert dummy embeddings in `knowledge_embeddings`, invoke service `delete`, and assert 0 embeddings remain in database.
- **Job Tests:** Verify `record_day_log` and `record_footprint` enqueue jobs that run to completion and populate `knowledge_embeddings`.
- **Search Verification:** Hybrid search query returns hits across notes, bookmarks, journal entries, and travel footprints in a single combined ranking.

---

## Blueprint 9: Global Command Palette (`Cmd+K` / `Ctrl+K`)

### 1. Problem Statement & User Value
With 18 production frontend modules registered in [`App.tsx:41-45`](../../../frontend/src/App.tsx#L41-L45) plus two standalone views (`LinkPage` and `PublicTransparencyView`, totaling 20 web surfaces), navigation requires repetitive sidebar clicking. Power users have no centralized shortcut to jump across views, execute frequent quick actions (e.g. creating a task, logging an expense, recording a bookmark), or run instant cross-domain searches.

### 2. Codebase Evidence & Grounding
- **Frontend Module Registry:** `frontend/src/core/registry.ts`: `registry.getAll()` provides access to all registered module metadata.
- **Registered Modules:** `frontend/src/App.tsx` lines 41–45.
- **Navigation Groups:** `frontend/src/components/Sidebar.tsx` lines 35–72 (`NAV_GROUPS`).
- **AI Prompt Bus:** `frontend/src/modules/ai/aiPromptBus.ts`: Already implements a window event listener pattern for routing prompts across modules.
- **Design System:** [`Design.md`](../../../Design.md): Strict monochrome token system, Geist Sans / Geist Mono typography, zero card-in-card or decorative gradients, explicit focus outlines, and `Escape` modal closing.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │            User Press: Cmd+K or Ctrl+K                 │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │              Command Palette Modal                     │
 │  Input with debounced instant fuzzy evaluation         │
 └───────────────────────────┬────────────────────────────┘
                             │
     ┌───────────────────────┼────────────────────────┐
     ▼                       ▼                        ▼
 ┌──────────────┐     ┌──────────────┐         ┌──────────────┐
 │ Tier 1: Nav  │     │ Tier 2: Act  │         │ Tier 3: Find │
 │ Jump to any  │     │ Fast Modals  │         │ Global Hybrid│
 │ of 20 views  │     │ (Todo, Exp)  │         │ Search Query │
 └──────────────┘     └──────────────┘         └──────────────┘
```

#### Domain Layer (`frontend/src/components/CommandPalette/`)
- Add `PaletteAction`: Action metadata with identifier, title, subtitle, category (`Navigation`, `QuickAction`, `InstantSearch`), shortcut representation, and execution handler.
- Fuzzy matching algorithm: Matches input substring and token prefixes, ranking exact matches, prefix matches, and fuzzy matches in deterministic order.

#### Database Layer
- Evaluation: Stateless, client-side action catalog. Server-side searches leverage existing PostgreSQL indexes via the hybrid search REST API.

#### Application Layer
- Integrates with existing application capabilities:
  - Todo creation: Dispatches to `TodoService::create_todo`.
  - Expense creation: Dispatches to `FinanceService::record_expense`.
  - Bookmark creation: Dispatches to `BookmarkService::create_bookmark`.
  - Hybrid search: Dispatches to `SearchService::hybrid_search`.

#### Interface Layer
- REST APIs consumed:
  - `GET /api/v1/search/hybrid?query=...` (debounced 200ms query).
  - Existing POST endpoints for quick modal actions.

#### Frontend Layer (`frontend/src/components/CommandPalette/`)
- `CommandPaletteModal.tsx`: Accessible ARIA combobox dialog with backdrop blur and focus trapping.
- `useCommandPalette.ts`: Custom hook managing keyboard shortcuts (`Cmd+K`, `Ctrl+K`, `Escape`, arrow navigation, selection).
- `paletteRegistry.ts`: Defines action catalog across three tiers:
  1. **Navigation Items:** Generated dynamically from `registry.getAll()` plus standalone views (20 navigable views).
  2. **Quick Actions:**
     - `"Create Todo"` -> Opens `AddTodoModal` or emits bus event.
     - `"Record Expense"` -> Opens `AddTransactionModal`.
     - `"Capture Note"` -> Navigates to `/knowledge` with new note draft.
     - `"Trigger Agent Task"` -> Navigates to `/agent` and triggers modal.
     - `"Toggle Dark Mode"` -> Invokes `applyTheme`.
  3. **Instant Search:** Debounced (200ms) query against `GET /api/v1/search/hybrid?query=...` showing matching notes, bookmarks, journal entries, and footprints.

#### Keyboard Interaction Invariants
- `Cmd+K` (macOS) / `Ctrl+K` (Windows/Linux) toggles palette open/closed.
- `Escape` dismisses the palette and restores prior document focus.
- Up/Down arrow keys navigate list items with active item highlight.
- `Enter` executes the selected action or navigation target.

### 4. Invariants & Safety Constraints
- **Performance:** Debounced search queries to avoid hammering backend endpoints during fast typing.
- **Accessibility:** Screen-reader accessible with `aria-expanded`, `aria-activedescendant`, and focus trapping.
- **Design Adherence:** Strictly follows [`Design.md`](../../../Design.md) typography and monochrome color tokens.

### 5. Verification & Test Strategy
- **Unit Tests:** Palette action filter and shortcut handler logic.
- **Playwright E2E:** Press `Cmd+K`, verify modal opens, search for "Finance", press Enter, verify route navigates to Finance module.
- **Accessibility Tests:** Verify focus trapping and Escape key behavior.

---

## Blueprint 10: Autonomous Scheduled Agent Task Controls

### 1. Problem Statement & User Value
The autonomous agent runtime features extensible scheduled tasks (e.g. `WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`). While tasks can be registered and listed:
1. Users cannot pause or resume a scheduled task without deleting its configuration ([`interface/http/agent.rs`](../../../backend/src/interface/http/agent.rs)).
2. Users cannot trigger an immediate out-of-schedule execution ("Run Now").
3. Users cannot delete a scheduled task.
4. In the web UI, [`ScheduledTasksTab.tsx:15-23`](../../../frontend/src/modules/agent/components/ScheduledTasksTab.tsx#L15-L23) has a dead `handleToggle` function that is not connected to any UI control.

### 2. Codebase Evidence & Grounding
- **Database Schema:** `scheduled_agent_tasks` table has columns `enabled BOOLEAN NOT NULL DEFAULT TRUE`, `next_run_at TIMESTAMPTZ`, `last_run_at TIMESTAMPTZ` ([`0009_agent_runtime.sql:5-24`](../../../backend/migrations/0009_agent_runtime.sql#L5-L24)).
- **Repository Implementation:** `backend/src/infrastructure/postgres/repositories/agent/tasks.rs`:
  - Contains `upsert_scheduled_task`, `list_scheduled_tasks_for_account`, `find_scheduled_task`, `list_due_scheduled_tasks`, `update_task_next_run`.
  - Lacks delete and toggle functions!
- **Application Service:** `backend/src/application/agent/tasks.rs`:
  - `schedule_task` always sets `enabled: true`. Lacks pause, resume, trigger, and delete methods.
- **Frontend Tab:** `frontend/src/modules/agent/components/ScheduledTasksTab.tsx`:
  - Has unused `handleToggle` function. Card renders static "Active/Disabled" badge without action buttons.

### 3. Architectural Design

```text
 ┌────────────────────────────────────────────────────────┐
 │                    Web Agent Console                   │
 │  Card Action Buttons:                                  │
 │  [Toggle Pause/Resume]  [Run Now (Play)]  [Delete]     │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │            Axum HTTP Interface (http/agent.rs)         │
 │  PATCH  /api/v1/agent/tasks/:id/state  {"enabled": ...}│
 │  POST   /api/v1/agent/tasks/:id/trigger                │
 │  DELETE /api/v1/agent/tasks/:id                        │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │               AgentService (application)               │
 │  - pause/resume: updates enabled & recomputes next_run │
 │  - trigger_now: creates AgentRun & enqueues job        │
 │  - delete: removes scheduled_agent_tasks row           │
 └───────────────────────────┬────────────────────────────┘
                             │
                             ▼
 ┌────────────────────────────────────────────────────────┐
 │          PostgreSQL Repository (agent/tasks.rs)        │
 │  Atomic state updates & concurrency protection         │
 └────────────────────────────────────────────────────────┘
```

#### Domain Layer (`backend/src/domain/agent/`)
- `TaskId` validation: 3 to 64 characters, lowercase alphanumeric, `_`, `-`.
- `CronSchedule`: Validates 5-field/6-field cron expressions.
- Task state machine: `ScheduledTaskState`: `Active <-> Paused`.
- Monotonic Next Run Invariant: Resuming a paused task recalculates `next_run_at` strictly after current wall-clock time (`now`), preventing historical burst executions.
- Agent Run Concurrency Invariant: At most 1 active agent run per account; attempting immediate trigger when a run is active returns `AgentError::ActiveRunConflict`.

#### Database Layer
- Table: `scheduled_agent_tasks` (`0009_agent_runtime.sql`).
- Unique constraint: `uq_account_task (account_id, task_id)`.
- Partial index: `idx_scheduled_agent_tasks_due ON scheduled_agent_tasks(next_run_at) WHERE enabled = TRUE;`.
- Registered in `OWNED_TABLES` in `backend/src/domain/accounts/mod.rs`.

#### Application Layer (`backend/src/application/agent/tasks.rs`)
- `AgentService::set_task_enabled(account_id, task_id, enabled) -> Result<ScheduledTaskDefinition, AppError>`:
  - When resumed (`enabled = true`), recomputes `next_run_at` from `cron_expression` starting from `now`.
  - Publishes `SystemEvent::AgentTaskUpdated`.
- `AgentService::trigger_task_now(account_id, task_id) -> Result<AgentRun, AppError>`:
  - Asserts no active run exists for account (`find_active_run_for_account`).
  - Creates `AgentRun` with status `Planning`.
  - Enqueues `JobType::AgentExecution`.
  - Publishes `SystemEvent::AgentRunCreated`.
- `AgentService::delete_task(account_id, task_id) -> Result<(), AppError>`:
  - Deletes row from `scheduled_agent_tasks`.
  - Publishes `SystemEvent::AgentTaskDeleted`.

#### Interface Layer (`backend/src/interface/`)
- **Repository Additions (`backend/src/infrastructure/postgres/repositories/agent/tasks.rs`):**
  ```rust
  pub async fn set_scheduled_task_state<'e>(
      conn: impl PgExecutor<'e>,
      account_id: &AccountId,
      task_id: &TaskId,
      enabled: bool,
      next_run_at: DateTime<Utc>,
  ) -> Result<bool, sqlx::Error>

  pub async fn delete_scheduled_task<'e>(
      conn: impl PgExecutor<'e>,
      account_id: &AccountId,
      task_id: &TaskId,
  ) -> Result<bool, sqlx::Error>
  ```
- **REST Endpoints (`backend/src/interface/http/agent.rs`):**
  - `PATCH /api/v1/agent/tasks/{id}/state`: Takes `{ "enabled": bool }`, returns updated `ScheduledTaskResponse`.
  - `POST /api/v1/agent/tasks/{id}/trigger`: Enqueues immediate run, returns `AgentRunResponse`.
  - `DELETE /api/v1/agent/tasks/{id}`: Returns `204 No Content`.
- **Chat Commands (`backend/src/interface/chat/agent.rs`):**
  - `!agent task pause <id>`, `!agent task resume <id>`, `!agent task run <id>`, `!agent task delete <id>`.

#### Frontend Layer (`frontend/src/modules/agent/`)
In `ScheduledTasksTab.tsx`:
- Add Pause / Resume switch toggle calling `PATCH /state`.
- Add "Run Now" button (PlayCircle icon) calling `POST /trigger` with loading spinner.
- Add "Delete" button (Trash2 icon) with confirmation alert calling `DELETE`.

### 4. Invariants & Safety Constraints
- **Active Run Invariant:** Immediate trigger enforces the rule of at most 1 active agent run per account, returning `409 Conflict` if a run is already in progress.
- **Monotonic Next Run:** Resuming a paused task MUST recalculate `next_run_at` strictly after current wall-clock time to avoid executing past missed schedules in a burst.
- **Step-Up Authentication:** Task deletion and state mutations audit `account_id` and require an active session.

### 5. Verification & Test Strategy
- **Unit Tests:** Cron recalculation on task resume to prevent back-execution storms.
- **Repository Tests:** Verify `set_scheduled_task_state` and `delete_scheduled_task` respect owner isolation.
- **Application Tests:** Triggering immediate run fails with `ActiveRunConflict` when another run is executing.
- **E2E Tests:** Toggle pause switch in web UI, verify state updates to "Disabled", trigger "Run Now", and observe run appear in live execution drawer.

---

## Architectural Comparison Matrix

| Blueprint | Primary Layer | Core Invariant / Boundary | Storage Category / Table | Verification Gates |
|---|---|---|---|---|
| **1. Web AI Phase 2/3** | Presentation & Session | Owner isolation; 2 concurrent stream cap | `ai_conversations.title` | Unit, SSE integration, Playwright |
| **2. Universal Calendar** | Domain Serialization | RFC 5545 format; reminder source deduplication | `account_calendar_tokens` | RFC parser check, repository tests |
| **3. Omnichannel Audio** | Inbound Gateway | Max 10 MiB; fail-open STT degradation | Temp memory buffer / Whisper STT | Inbound mock tests, format checks |
| **4. Email Ingestion** | Channel Inbound | DMARC validation pass; 10 MiB limit | `StorageCategory::Receipts` | RFC 5322 fixture tests, job tests |
| **5. Receipt Viewer** | REST & Storage | Presigned HMAC 300s TTL; owner check | `transactions.receipt_storage_key` | Presign URL tests, E2E lightbox |
| **6. Briefing Podcast** | Background Jobs | Fail-open TTS; 2,000 char script cap | `StorageCategory::Audio` | Audio compilation tests, web player |
| **7. Second Brain Links** | Domain & Relational | Max 50 links/note; self-link rejection | `knowledge_note_links` | Pure parser tests, graph integration |
| **8. Multi-Domain Search** | Knowledge & Vector | Transactional embedding deletion | `knowledge_embeddings` | Deletion leak tests, hybrid query |
| **9. Command Palette** | Web Frontend | Keyboard accessibility; debounced search | In-memory registry / hybrid API | Component unit tests, E2E shortcut |
| **10. Task Controls** | Agent & Scheduler | Max 1 active run per account; monotonic cron | `scheduled_agent_tasks` | Concurrency tests, API roundtrips |

---

## Implementation Sequence & Dependencies

```text
  Phase A: Critical Data Integrity & Core Usability (Immediate)
  ├── Blueprint 8: Fix Vector Embedding Deletion Leaks & Multi-Domain Search
  ├── Blueprint 1: Web AI Assistant Phase 2 (Thread Switcher, Non-Destructive New Chat)
  └── Blueprint 10: Autonomous Agent Task Controls (Pause, Resume, Trigger, Delete)

  Phase B: Omnichannel Parity & Financial Automation
  ├── Blueprint 5: Scanned Receipt Preservation & Web Lightbox Viewer
  ├── Blueprint 4: Inbound Email Receipt Ingestion & OCR Pipeline
  └── Blueprint 3: Omnichannel Audio Ingress Parity (Telegram & Discord STT)

  Phase C: Personal Productivity & Knowledge Management
  ├── Blueprint 2: Universal Calendar Feed (RFC 5545 iCalendar / Webcal)
  ├── Blueprint 7: Second Brain Bidirectional Wiki-Links & Visual Graph
  ├── Blueprint 6: Scheduled Morning Briefing Podcast (Kokoro TTS)
  └── Blueprint 9: Global Command Palette Across All 20 Modules
```

---

*End of specification.*
