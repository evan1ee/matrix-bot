# Inbound Email Receipt Ingestion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A DMARC-passing email from a linked sender with receipt images attached is stored, OCR'd, recorded as an expense and answered by email, exactly once per attachment.

**Architecture:** `parse_inbound_email` gains pure attachment extraction (`email/attachments.rs`). The Email adapter routes a message that carries attachment parts to a new `email/receipts.rs` ingest step (resolve owner → store in `Receipts` → enqueue `media.process_receipt` with a per-attachment dedupe key → reply). The existing `ProcessReceiptHandler` does OCR, `record_expense` and the reply through `Outbound`, which already reaches `ConversationAddress::Email`.

**Tech Stack:** Rust, `mail-parser` 0.11, SQLx jobs table, existing `StorageService` and `OcrClient`.

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` (Blueprint 4)

## Global Constraints

- DMARC must pass: `parse_inbound_email` already errors before attachments are read; keep that order.
- Max 10 MiB per attachment, max 30 MiB per message.
- Job dedupe key is `email_receipt:{message_id}:{attachment_idx}`.
- Store under `StorageCategory::Receipts`; money stays integer minor units.
- Interface code runs no SQL; enqueue goes through `repositories::jobs::enqueue`, as the Discord adapter does.
- No file over 500 production lines (`inbound.rs` is ~400, so new code goes in new files).

## Rulings (recorded in HANDOFF)

- **PDFs are not OCR'd.** Neither the backend nor `inference/` can decode PDFs, and a rasteriser is a heavy dependency for one input type. PDFs are extracted, skipped and named in the reply. Same call as Blueprint 5. Cost if wrong: PDF-only invoices need manual entry until a rasteriser lands; the 3-page bound becomes moot until then.
- **Type comes from magic bytes**, not the declared MIME type (`sniff_receipt_type`).
- **A message with attachment parts is a receipt submission**: its body is the caption and is not dispatched as a command or AI prompt.
- **Inline parts with a `Content-ID`** (HTML logos, tracking pixels) are ignored.
- **At most 5 receipts per message**, so one email cannot fan out into hundreds of OCR jobs.
- **No "Email" origin badge.** `source_kind` only models recurring bills and portfolio cash; a badge needs a new source kind, a migration and an OpenAPI change. The paperclip from Blueprint 5 already opens the receipt.

## Review Focus

- Receipt email with no body and no subject must still be ingested (today it is dropped as `EmptyBody`).
- PDF-only email: sender gets a reply saying PDFs are unsupported, not silence.
- Same message fetched twice (restart clears the in-memory window): one job per attachment.
- Embedded logo with `Content-ID` is not OCR'd.
- Unlinked sender gets `UNLINKED_REPLY` and nothing is stored.
- Attachment whose declared type is `image/png` but whose bytes are text is rejected.

---

### Task 1: Per-attachment job key

**Files:**
- Modify: `backend/src/domain/jobs.rs` (next to `media_process_receipt`, ~line 276; test beside line 606)

**Interfaces:**
- Produces: `NewJob::media_process_email_receipt(owner: AccountId, storage_key: StorageKey, conversation: ConversationAddress, caption: Option<String>, message_id: &str, attachment_idx: usize) -> NewJob`; job type `MediaProcessReceipt`, 3 attempts, 60 s backoff, dedupe key `email_receipt:{message_id}:{attachment_idx}`.

- [ ] **Step 1:** Test `email_receipt_job_dedupes_by_message_and_index`: same message/index with two different storage keys gives equal `dedupe_key()`; index 0 vs 1 differ; payload is `MediaProcessReceipt` carrying the conversation.
- [ ] **Step 2:** Run `cargo test -p matrix-bot email_receipt_job` — expect compile failure.
- [ ] **Step 3:** Implement by extracting the shared body of `media_process_receipt` into a private helper taking the dedupe key.
- [ ] **Step 4:** Run the test — expect PASS. Commit `feat: per-attachment dedupe key for email receipt jobs`.

### Task 2: Attachment extraction

**Files:**
- Create: `backend/src/interface/channels/email/attachments.rs`, `contracts/channels/email_receipt_png.eml`, `email_receipt_pdf_only.eml`, `email_receipt_inline_logo.eml`
- Modify: `email/mod.rs`, `email/inbound.rs` (`ParsedEmail`, empty-body rule), `interface/http/finance_receipts.rs` (nothing but already `pub(crate)`)

**Interfaces:**
- Produces:
  - `pub struct InboundAttachment { pub filename: String, pub content_type: &'static str, pub extension: &'static str, pub data: Vec<u8> }`
  - `pub struct SkippedAttachment { pub filename: String, pub reason: SkipReason }`; `pub enum SkipReason { Pdf, TooLarge, MessageTooLarge, TooMany, NotAReceipt }` with `fn describe(&self) -> &'static str`
  - `pub struct Extracted { pub accepted: Vec<InboundAttachment>, pub skipped: Vec<SkippedAttachment> }`
  - `pub fn extract_attachments(msg: &mail_parser::Message) -> Extracted`
  - consts `MAX_ATTACHMENT_BYTES = 10 MiB`, `MAX_MESSAGE_BYTES = 30 MiB`, `MAX_RECEIPTS_PER_MESSAGE = 5`
  - `ParsedEmail` gains `attachments: Vec<InboundAttachment>` and `skipped_attachments: Vec<SkippedAttachment>`; an empty body with any attachment part yields empty `FreeText` instead of `EmptyBody`.

- [ ] **Step 1:** Fixtures: RFC 5322 multipart with a real 1×1 PNG (base64) and DMARC pass; PDF-only; HTML + inline `Content-ID` logo. Tests in `attachments.rs`: png accepted with `image/png`/`png`; PDF skipped as `Pdf`; mislabelled text part skipped `NotAReceipt`; inline logo ignored entirely; 11 MiB part skipped `TooLarge`; four 9 MiB parts → fourth skipped `MessageTooLarge`; six tiny images → sixth `TooMany`. In `inbound.rs`: png fixture parses with one attachment and no body; DMARC-fail fixture with an attachment still returns `Err(Dmarc)`.
- [ ] **Step 2:** Run `cargo test -p matrix-bot channels::email` — expect failures.
- [ ] **Step 3:** Implement `extract_attachments` over `msg.attachments()`, using `sniff_receipt_type` and `Content-ID` check; wire into `parse_inbound_email`.
- [ ] **Step 4:** Run tests — PASS. Commit `feat: extract receipt attachments from inbound email`.

### Task 3: Ingest step and adapter wiring

**Files:**
- Create: `backend/src/interface/channels/email/receipts.rs`
- Modify: `email/adapter.rs` (`poll_once` branch), `email/mod.rs`

**Interfaces:**
- Consumes: Task 1 and Task 2 items; `Inbound::services()`, `Inbound::db()`, `identity.resolve`.
- Produces: `pub async fn ingest_receipts(inbound: &Inbound, parsed: &ParsedEmail) -> Reply` — never panics, always returns the text to mail back.

- [ ] **Step 1:** DB-backed tests (`test_pool`, `LocalStorageService`): linked sender with one PNG → one object under `accounts/{owner}/receipts/`, one job with the Task 1 key, reply says queued; calling twice → still one job; unlinked sender → `UNLINKED_REPLY`, no object, no job; PDF-only → reply names the file and says PDFs are unsupported, no job; storage failure → reply asks to retry, no job.
- [ ] **Step 2:** Run — expect compile failure.
- [ ] **Step 3:** Implement: resolve owner; per accepted attachment `StorageKey::generate(owner, Receipts, ext)` → `storage.put` → `enqueue`; build the reply from counts plus `SkipReason::describe`. In `poll_once`, after the dedupe check, call it when `attachments` or `skipped_attachments` is non-empty and send the reply by the existing `send_message` path; otherwise unchanged.
- [ ] **Step 4:** Run tests — PASS. Commit `feat: ingest emailed receipts through the receipt job`.

### Task 4: Handler reply over email, docs, gates

**Files:**
- Modify: `backend/src/interface/jobs/media.rs` tests; `docs/superpowers/HANDOFF.md`; `docs/architecture.md` (channels section, one paragraph)

- [ ] **Step 1:** Test in `media.rs`: a job whose conversation is `ConversationAddress::Email` with OCR unconfigured completes and the fake outbound records one send to that address.
- [ ] **Step 2:** Run it; fix `FakeOutbound` assertions only if the helper is Matrix-specific.
- [ ] **Step 3:** HANDOFF rulings block (above) plus a note that the handler parses the amount through `f64` (pre-existing, shared with Matrix and Discord) against the integer-money rule.
- [ ] **Step 4:** Gates: `cargo fmt --all -- --check`, `cargo check -p matrix-bot --all-targets`, `cargo clippy -p matrix-bot --all-targets -- -D warnings`, `cargo test -p matrix-bot --all-targets` (test Postgres on :55433, `--test-threads=2`), `git diff --check`. No query or schema change, so no `.sqlx/` or OpenAPI regeneration.
- [ ] **Step 5:** Commit `docs: record email receipt ingestion progress`.
