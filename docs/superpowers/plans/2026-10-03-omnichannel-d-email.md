# Sub-project 2, Plan D: Email adapter — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Status:** approved 2026-10-03; in progress (HANDOFF §4 Plan D).

**Goal:** Email becomes the fourth `ChannelAdapter`. An email user can send commands (e.g. `/todo`, `/expense`, `/portfolio`, `/help`, `/link`) or converse with the AI agent over email. Outbound replies, reminders, briefings, and link confirmations are delivered via SMTP (`lettre`) from `EMAIL_FROM`. Inbound mail is polled via IMAPS (`async-imap`) from `EMAIL_IMAP_URL`, authenticated against DMARC (`Authentication-Results` from `EMAIL_AUTHSERV_ID`), stripped of quotes and signatures, and dispatched through the shared `Inbound` pipeline. Auto-replies and unauthenticated emails are dropped without reply to eliminate backscatter spam.

**Architecture:**
- **Config** `config/channels.rs` — `EmailConfig { imap_url, smtp_url, username, password, from, authserv_id, poll_seconds, processed_folder }` from `EMAIL_*` env vars. The adapter exists only when `EMAIL_IMAP_URL` is set.
- **Inbound** `interface/channels/email/inbound.rs` — RFC 5322 MIME parsing (`mail-parser`), DMARC authentication verification (Invariant 8), auto-reply detection, quote and signature stripping, command parsing.
- **Outbound** `interface/channels/email/outbound.rs` — `OutgoingMessage` -> MIME multipart (`text/plain` + `text/html`) with `In-Reply-To`/`References`, action list rendering, SMTP submission via `lettre`.
- **IMAP Polling** `interface/channels/email/imap.rs` — IMAPS connection via `async-imap` over `tokio-rustls`, search `UNSEEN`, bounded fetch, mark `\Seen`, move to processed folder if configured.
- **Adapter** `interface/channels/email/adapter.rs` — implements `ChannelAdapter`, wraps polling loop and SMTP sender, checks account delivery authorization.
- **Composition root** `app.rs` — builds `EmailAdapter` when configured, attaches to shared pipeline, registers in `ChannelRegistry`, supervised as "Email polling".

No migration, no new table, no SQL, no service, no `/api/v1` route, no frontend change, `backend/openapi.json` byte-identical.

**Tech Stack:** Rust, `mail-parser` 0.11, `lettre` 0.11 (tokio1-rustls), `async-imap` 0.12 (tokio), `tokio-rustls` 0.26, tokio + `tokio_util::sync::CancellationToken`.

**Spec:** `docs/superpowers/specs/2026-10-01-omnichannel-design.md` (§3.2–§5, §8.4, §9–§12, invariant 8).

---

## Code this plan builds on (read before Task 1)

| Path | What it gives Email |
|---|---|
| `backend/src/domain/channels/mod.rs` | `Channel::Email`, `PlatformUserId::Email` (lowercased address-spec), `ConversationAddress::Email(addr)`, `RequireLink` policy |
| `backend/src/config/channels.rs` | `parse_link_signup_allowlist` already parses `EMAIL_ALLOWED_SENDERS`; pattern for per-channel config and validation |
| `backend/src/interface/channels/adapter.rs` | `ChannelAdapter`, `ChannelError { NotConnected, NotConfigured, Rejected, Transient }`, `SentRef` |
| `backend/src/interface/channels/message.rs` | `IncomingMessage`, `OutgoingMessage`, `Content`, `Action`, `Reply`, `Visibility`, `MessageRef` |
| `backend/src/interface/channels/inbound.rs` | `Inbound::handle` -> `InboundOutcome`; unlinked filter; command vs free text dispatch |
| `backend/src/interface/chat/link.rs` | `handle_link` returns private reply with link URL; email receives it as direct email |
| `backend/src/app.rs` | `AdapterTask` supervision, `front_door_name(Email) == "Email polling"` |

---

## Global Constraints

- **Layers.** `lettre`, `async_imap`, and `mail_parser` appear only under `interface/channels/email/` (enforced by `tests/architecture.rs`). `application/`, `llm/`, `interface/chat/`, `domain/` stay free of them. `interface/channels/**` runs no SQL.
- **Frozen behaviour elsewhere.** Every Matrix, Discord, and Telegram command, reply, job and outbox behaviour is unchanged.
- **OpenAPI.** `backend/openapi.json` must remain byte-identical after every task.
- **No network in tests.** Pure functions over RFC 5322 fixtures under `contracts/channels/`. Tests never contact an IMAP or SMTP server.
- **Security Invariant 8:** An email without `dmarc=pass` from the configured `authserv_id` is never processed and never answered (drops silently to prevent backscatter spam).
- **Auto-replies:** Messages with `Auto-Submitted` != "no", `Precedence: bulk/list/junk`, or from `mailer-daemon`/`postmaster`/`noreply` are dropped silently.
- **Bounds:** IMAP poll fetches at most 20 messages per run. Body text for commands capped at 500 characters. SMTP sends have explicit 30s timeouts. Recent `Message-ID`s deduplicated in a 1024-entry sliding window.
- **File size:** No file exceeds 500 production lines (target < 300).
- **Error handling:** No `unwrap()`/`expect()` on runtime paths. All library errors map to `ChannelError` at the boundary.

---

## Plan clarifications (rulings E1–E14 / R118–R131)

- **E1 (R118) — Dependencies:** `mail-parser` 0.11 for MIME parsing, `lettre` 0.11 (`default-features = false, features = ["smtp-transport", "tokio1-rustls-tls", "builder", "tracing"]`) for SMTP, `async-imap` 0.12 (`default-features = false, features = ["runtime-tokio", "tokio"]`) for IMAP, and `tokio-rustls` 0.26 / `webpki-roots` 0.26 for IMAP TLS.
- **E2 (R119) — Inbound text extraction:** Body extracted from first `text/plain` part (falls back to `text/html` converted to text). Quotes (`> ...`, `On ... wrote:`, `-----Original Message-----`) and signatures (`-- `) stripped. If body is empty after stripping, falls back to `Subject`. Commands can be typed with `/`, `!`, or bare command name matching registry.
- **E3 (R120) — Inbound authentication (DMARC pass rule, Invariant 8):** The adapter parses all `Authentication-Results` headers. Only the header whose authserv-id matches `EMAIL_AUTHSERV_ID` is trusted. It must contain `dmarc=pass` for the `From` domain. Anything else is logged with `warn!` and dropped without a reply (no backscatter).
- **E4 (R121) — Auto-reply drop:** Messages with `Auto-Submitted` != "no", `Precedence: bulk/list/junk`, or `From` starting with `mailer-daemon`, `postmaster`, `noreply`, `no-reply` are dropped without reply.
- **E5 (R122) — Identity & Conversation mapping:** Sender is `PlatformIdentity(Channel::Email, PlatformUserId::parse(Channel::Email, addr)?)` with display name from `From`. Conversation is `ConversationAddress::Email(addr)`. `is_direct = true`, `is_mentioned = true`.
- **E6 (R123) — Outbound threading & Subject:** Outgoing replies set `In-Reply-To` and `References` to the incoming `Message-ID`. Subject is `Re: <subject>` (preserves existing `Re:`). Unsolicited messages derive subject from first line of text or `"Matrix Bot"`.
- **E7 (R124) — Outbound multipart & Action rendering:** Messages are sent as multipart/alternative (`text/plain` + `text/html`). `Content.actions` render as a text list at the bottom: "Reply to this email with: • <command>".
- **E8 (R125) — Linking:** `/link` sends the token URL to the `From` address. Claiming the link publishes `IdentityLinked` which triggers an outbox confirmation email "✓ linked" back to `email:<address>`.
- **E9 (R126) — IMAP polling:** Polling loop connects to `EMAIL_IMAP_URL` (IMAPS, port 993 default), logs in with `EMAIL_USERNAME`/`EMAIL_PASSWORD`, selects `INBOX`, searches `UNSEEN`, fetches at most 20 messages per poll, marks `\Seen`, and optionally moves to `EMAIL_PROCESSED_FOLDER`.
- **E10 (R127) — SMTP transport:** Lettre `AsyncSmtpTransport` with STARTTLS or implicit TLS. Timeouts capped at 30s. Errors mapped to `ChannelError::Transient` (temporary 4xx or connection loss) or `ChannelError::Rejected` (permanent 5xx).
- **E11 (R128) — Progress:** Email uses `llm::progress::NoProgress` (no-op; intermediate tool calls are not emailed).
- **E12 (R129) — Config validation:** Registered only when `EMAIL_IMAP_URL` is set. When set, `EMAIL_SMTP_URL`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`, `EMAIL_FROM`, and `EMAIL_AUTHSERV_ID` are required. `EMAIL_ALLOWED_SENDERS` validated as email addresses.
- **E13 (R130) — Deduplication:** Inbound messages deduplicated by `Message-ID` in a 1024-entry sliding window. Outbox deliveries to email are at-least-once.
- **E14 (R131) — Architecture isolation:** Architecture test verifies `lettre`, `async_imap`, and `mail_parser` are isolated inside `interface/channels/email/`.

---

## File map

| File | Responsibility | Task |
|---|---|---|
| `backend/Cargo.toml` | Dependencies: `mail-parser`, `lettre`, `async-imap`, `tokio-rustls`, `webpki-roots` | 1 |
| `backend/src/config/channels.rs`, `config/mod.rs` | `EmailConfig`, `parse_email`, `validate_email`, `validate_email_allowlist` | 1 |
| `contracts/channels/email_*.eml` | RFC 5322 test fixtures (DMARC pass, fail, auto-reply, quotes) | 2 |
| `backend/src/interface/channels/email/inbound.rs` | MIME parsing, DMARC check, auto-reply filter, quote stripping, command normalization | 2 |
| `backend/src/interface/channels/email/outbound.rs` | MIME multipart builder, action renderer, SMTP transport, error classification | 3 |
| `backend/src/interface/channels/email/imap.rs` | IMAPS client, polling loop, `UNSEEN` fetch, folder move, backoff | 4 |
| `backend/src/interface/channels/email/adapter.rs`, `mod.rs` | `EmailAdapter` implementing `ChannelAdapter` | 5 |
| `backend/src/app.rs`, `tests/architecture.rs` | Supervisor wiring, registry registration, architecture boundary assertions | 6 |
| `docs/architecture.md`, `README.md`, `.env.example`, `SPEC.md`, `HANDOFF.md` | Documentation, configuration reference, hand-off state | 7 |

---

## Tasks

### Task 1: Dependencies and Email Configuration
- [ ] Add dependencies to `backend/Cargo.toml`: `mail-parser = "0.11"`, `lettre = { version = "0.11", default-features = false, features = ["smtp-transport", "tokio1-rustls-tls", "builder", "tracing"] }`, `async-imap = { version = "0.12", default-features = false, features = ["runtime-tokio", "tokio"] }`, `tokio-rustls = { version = "0.26", default-features = false, features = ["ring"] }`, `webpki-roots = "0.26"`.
- [ ] In `backend/src/config/channels.rs`: define `EmailConfig` with redacted `Debug` for credentials; `parse_email`, `validate_email`, and `validate_email_allowlist`.
- [ ] In `backend/src/config/mod.rs`: add `pub email: Option<EmailConfig>` to `Config`, wire into `from_env_inner` and `validate`.
- [ ] Unit tests for parsing, redaction, and validation.
- [ ] Verification: Gate check (Task 1).

### Task 2: Inbound Normalization & DMARC Authentication
- [ ] Create fixtures under `contracts/channels/`:
  - `email_dmarc_pass.eml`: Valid email with matching `Authentication-Results` (`dmarc=pass`).
  - `email_dmarc_fail.eml`: Email with `dmarc=fail` or missing authserv header.
  - `email_autoreply.eml`: Auto-reply email (`Auto-Submitted: auto-replied`).
  - `email_quoted.eml`: Email with `> quote`, attribution line, and `-- ` signature.
- [ ] Implement `backend/src/interface/channels/email/inbound.rs`:
  - `verify_dmarc`: parses `Authentication-Results` matching `authserv_id`, checks `dmarc=pass` for `From` domain.
  - `is_auto_reply`: checks `Auto-Submitted`, `Precedence`, and `From` prefix.
  - `strip_quotes_and_signatures`: cleans quoted lines, attribution, and signature.
  - `parse_inbound_email`: parses MIME, extracts `IncomingMessage` with `InboundText::Command` or `FreeText`.
- [ ] Unit tests covering all fixtures and edge cases.
- [ ] Verification: Gate check (Task 2).

### Task 3: Outbound Formatting & SMTP Send
- [ ] Implement `backend/src/interface/channels/email/outbound.rs`:
  - `render_email`: builds `lettre::Message` with multipart `text/plain` and `text/html`.
  - Subject derivation: `Re: <subject>` for replies, first line of text for unsolicited messages.
  - Actions rendering: "Reply to this email with: • <command>" plain text + HTML list.
  - `EmailSender`: wraps `lettre::AsyncSmtpTransport` with credentials and TLS.
  - Error classification: maps `lettre` errors to `ChannelError::Transient` or `ChannelError::Rejected`.
- [ ] Unit tests for MIME construction, header correctness, and error mapping.
- [ ] Verification: Gate check (Task 3).

### Task 4: IMAP Client & Polling Runtime
- [ ] Implement `backend/src/interface/channels/email/imap.rs`:
  - `connect_imaps`: establishes TLS TCP connection via `tokio-rustls` and creates `async_imap::Client`.
  - `ImapPoller`: logs in, selects `INBOX`, searches `UNSEEN`, fetches RFC822 messages (capped at 20).
  - Flags management: marks `\Seen`, moves to `EMAIL_PROCESSED_FOLDER` if set.
  - `poll_once` & polling loop with `CancellationToken` and backoff.
- [ ] Unit tests with pure seams.
- [ ] Verification: Gate check (Task 4).

### Task 5: Email Adapter Implementation
- [ ] Implement `backend/src/interface/channels/email/adapter.rs` and `mod.rs`:
  - `EmailAdapter` struct holding config, sender, and attached `Inbound` pipeline.
  - Implements `ChannelAdapter`: `channel()`, `is_connected()`, `run()`, `send()`, `authorize_delivery()`.
  - Inbound deduplication via 1024-entry sliding window.
  - `llm::progress::NoProgress` for progress reports.
- [ ] Unit tests for `ChannelAdapter` lifecycle and delivery authorization.
- [ ] Verification: Gate check (Task 5).

### Task 6: Composition Root & Architecture Boundaries
- [ ] In `backend/src/app.rs`:
  - Build `EmailAdapter` if `config.email` is present.
  - Attach shared `Inbound` pipeline via `Arc::new_cyclic`.
  - Register in `ChannelRegistry`.
  - Add to supervisor tasks under front door name `"Email polling"`.
- [ ] In `backend/tests/architecture.rs`:
  - Add `lettre`, `async_imap`, and `mail_parser` to forbidden lists for `application`, `domain/channels`, `llm`, and `interface/chat`.
  - Add `email_crates_stay_inside_the_email_adapter` assertion.
- [ ] Verification: Gate check (Task 6).

### Task 7: Documentation, Runbooks & Final Verification
- [ ] Update `docs/architecture.md` (Email adapter section).
- [ ] Update `README.md` (Configuration table, Enabling Email).
- [ ] Update `.env.example` (`EMAIL_*` settings commented out).
- [ ] Update `SPEC.md` (§9.4 Email).
- [ ] Update `docs/superpowers/HANDOFF.md` (record Plan D completion, rulings R118–R131).
- [ ] Full Gate verification suite:
  - `cargo fmt --all -- --check`
  - `cargo check -p matrix-bot --all-targets`
  - `cargo test -p matrix-bot --all-targets`
  - `cargo test -p matrix-bot live_postgres`
  - `cargo clippy -p matrix-bot --all-targets -- -D warnings`
  - `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets`
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
  - `git diff --check`
  - `cargo build -p matrix-bot --release`
  - `npm --prefix frontend run build`
