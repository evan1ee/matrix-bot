# Agent Handoff — Production Cutover & Remaining Roadmap

**Read this before touching code.** This document is the authoritative hand-off state for `matrix-bot`. It outlines the current state of the platform, immediate operational cutover actions, remaining roadmap enhancements, and settled architectural invariants.

> **Current Status (2026-10):** All major subsystems across the PostgreSQL foundation, omnichannel architecture, storage decoupling, autonomous agent runtime, web console polish, and **Second Brain & Knowledge Base (pgvector + hybrid search)** are **fully implemented and verified**.

---

## 1. Quick Start & Architecture Overview

`matrix-bot` is an **Omnichannel Personal AI/Life Workspace & Second Brain** built around Matrix, Discord, Telegram, Email, and a React 19 Web Console, powered by a modular Rust backend and local ONNX inference server.

- `backend/` (`matrix-bot`): Matrix bot, Axum REST/SSE API, MCP server, LLM gateway, durable jobs, transactional outbox, knowledge base (pgvector), and domain capabilities.
- `inference/` (`matrix-bot-inference`): Axum server hosting local ONNX models for OCR, speech-to-text (Sherpa/SenseVoice), and text-to-speech (Kokoro-82M).
- `frontend/` (`web`): React 19 + Vite + Tailwind v4 SPA, built into `backend/web/dist/`.
- `contracts/`: JSON wire fixtures asserting cross-platform compatibility.

### Layer-First Architecture (`backend/src/`)

```text
domain/<f>/                  entities, value objects, invariants, domain errors (pure, synchronous)
domain/channels/             Channel, PlatformIdentity, ConversationAddress, neutral messages
application/<f>/             <F>Service: use cases, transactions, publishes outbox events in tx
interface/http/              Axum REST handlers + DTOs (OpenAPI contracts)
interface/chat/              BotModule command handlers per feature, shared error replies
interface/channels/          ChannelAdapter trait, ChannelRegistry, shared inbound pipeline
interface/channels/matrix/   Matrix adapter: matrix-sdk client, sync loop, E2EE, verification
interface/channels/discord/  Discord adapter: serenity client, slash commands, embeds
interface/channels/telegram/ Telegram adapter: teloxide client, webhook/polling, inline keyboards
interface/channels/email/    Email adapter: async-imap polling, lettre SMTP, DMARC validation
interface/mcp/               MCP protocol adapters (stdio & HTTP/SSE)
interface/jobs/              Durable job handlers (reminder.fire, briefing.send, agent.execute_run, etc.)
jobs/                        Job runtime: workers (SKIP LOCKED), heartbeat reaper, advisory scheduler
events/                      Events runtime: LISTEN outbox loop, SSE hub, delivery dispatcher, purge
llm/                         Prompts, tool loop, tool registry, agent engine with native tool calling (llm/agent/)
infrastructure/postgres/     repositories/<f>.rs (sqlx::query! only), read_models/, merge.rs
infrastructure/storage/      StorageService: Local filesystem (HMAC presigning) and S3/R2
infrastructure/inference/    HTTP clients: OCR, STT, TTS, LLM transport
app.rs / main.rs             Composition roots, supervision, graceful shutdown
```

---

## 2. Hard Safety Rules (Never Override)

1. **Never read or modify the production `.env` or `data/` directory.**
   - `data/` contains the active Matrix SQLite crypto store.
   - `.env` contains live credentials. Development and test runs use isolated test databases.
2. **Never run the live bot binary against production homeservers or external channels.**
   - The only permitted executable during development is `cargo run --bin generate-openapi` or automated test suites.
   - Live logins and external channel interactions must be performed by the human operator.
3. **Never push, merge, open a PR, or deploy without explicit user instructions.**
   - Gitea (`git@git.mbm.one:evan/matrix-bot.git`) is primary; GitHub (`origin`) is a mirror.
4. **Preserve Security Invariants:**
   - Passkeys are the exclusive web authentication mechanism; session tokens are stored as SHA-256 only.
   - Step-up verification (`VerifiedUser`) is strictly enforced for sensitive endpoints (`/account/*`, `/portfolio/trade`, `/agent/approvals/*`).
   - Sensitive commands are strictly refused in public rooms/channels; `/link` is DM-only with 10-minute single-use tokens.
   - Every owner-keyed table must be registered in `OWNED_TABLES` or `ACCOUNT_ID_FOLLOWS` (`domain/accounts/mod.rs`).
   - Every user-supplied string rendered into HTML must pass through `html_escape()`; input strings are capped at 500 characters.

---

## 3. Verified System Baseline

All milestones are verified green:
- **Rust Backend:**
  - `cargo check -p matrix-bot --all-targets`: Clean (zero errors, zero warnings).
  - `cargo clippy --workspace --all-targets -- -D warnings`: Clean.
  - `cargo test -p matrix-bot --lib`: 1340 passed / 0 failed.
  - `cargo test -p matrix-bot --test architecture`: 39 passed / 0 failed.
  - `cargo test -p matrix-bot live_postgres`: 3 passed / 0 failed.
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: Byte-identical (zero drift).
- **Inference Server:**
  - `cargo test -p matrix-bot-inference`: 74 passed / 0 failed.
- **Frontend SPA:**
  - `npm --prefix frontend run build`: Clean (1769 modules transformed, zero TypeScript errors).
  - Route-based code splitting: Initial production bundle reduced to 486 kB (122 kB gzipped), eliminating chunk size warnings.
  - `npm --prefix frontend run test:e2e`: 208 passed / 0 failed (100% green across all Playwright specs).

---

## 3a. LLM prompt hardening (branch `fix/llm-prompt-hardening`)

Plan: `docs/superpowers/plans/2026-10-05-llm-prompt-hardening.md` (approved 2026-10-05). One commit per phase.

| Phase | Scope | Status |
|---|---|---|
| 1 | Static prompt prefix plus short context; no owner id; room cap | Done (`fix(llm): split system prompt into stable prefix and short context`) |
| 2 | Quoted and tool content as data; confirm tainted sensitive calls; language-independent success guard | Done (`fix(llm): treat quoted and tool content as data; confirm tainted sensitive calls`) |
| 3 | Agent native tool calling, ReAct removed | Done (`refactor(agent)!: native tool calling for agent runs`) |
| 4 | Execute the approved action on resume | Not started |
| 5 | Docs sweep | Not started |

Rulings:

- Ruling: the agent moves to native tool calling and ReAct text parsing is removed (D1) — `AgentStep` already stores structured calls, so steps map onto `tool_calls` and `tool` messages with no migration or API change — a model without function calling cannot run agents, but plain text becomes the final answer so it fails safe.
- Ruling: an agent tool call whose arguments are not a JSON object is recorded as a failed step and the run continues; empty arguments mean `{}` (phase 3) — silently coercing garbage into a call is how a wrong tool input could execute, while ending the run on one typo wastes the whole goal, and `max_steps` bounds repeats — a model that never produces valid arguments burns its budget and the run fails at the step limit. The agent's tool results are wrapped in `<tool_output>` like chat's, so its prompt rule is true; the agent has no taint tracking because every sensitive tool already needs human approval.
- Ruling: one language-independent confirmation rule replaces the zh/en reply templates (D2): after a success, start with ✅ and a short bold title, then bullets of the tool's key fields, in the user's language — rendering replies in code would couple formatting to every tool — formatting may drift between models; easy to revert.
- Ruling: a single success-claim guard (no tool succeeded this turn but the reply contains ✅) replaces the English substring lists (D3) — it works in every language — a reply that claims success without the ✅ is not caught.
- Ruling: chat confirms only tainted sensitive calls (D4): the turn carries quoted context, or an earlier round ran a tool that returns outside content (`web_search`, `get_webhook_logs`, `knowledge_search`); the tool is refused with `confirmation_required` and the model asks the user — confirming every sensitive call adds friction to plain requests — an untainted but mistaken call still runs without asking.
- Ruling: `update_todo` is not a sensitive tool, while `cancel_reminder` and `update_my_settings` are (added after the phase 2 review) — editing a todo is too common to confirm each time and the friction would dominate, whereas cancelling a reminder or changing settings is silent and hard to notice — an injected quote could rewrite a todo's text without confirmation. Agent runs now also need approval for those two tools.
- Ruling: a reply to the bot's own message does not taint the turn, decided by `QuotedMessage::from_bot` from platform data (Telegram `from` user, Matrix replied-to event sender), never by display name or quote text — otherwise a "yes" to a confirmation question could never run the tool — a platform bug that mislabels a sender would let that quote through untainted; a failed fetch counts as not ours.
- Ruling: the success-claim guard looks only at a reply that starts with ✅ — that is the D2 confirmation format, and a mid-text ✅ in a fact-check is not a claim — a model that confirms without the leading ✅ is not caught.
- Ruling: OCR text is wrapped in `<ocr_text>` and the OCR turn is tainted, `get_my_bookmarks` counts as outside content, and every `<` inside wrapped content is escaped — a photo or a web-fetched title can carry instructions, and matching only closing tags missed spaced and full-width variants — deletions and trades after a receipt scan now need a confirming reply; voice transcripts are still unwrapped.
- Ruling: dynamic context goes at the end of the single system message and a custom `system_prompt` still replaces everything (D5) — the static prefix stays byte-stable for provider caching — the prefix alone is only about 600 tokens, so the cache hit relies on the tools array being stable too; the conditional `web_search` tool breaks it.

---

## 4. Remaining Tasks & Action Plan

### Track 1: Production Cutover & Deployment (Human Operator / User-Gated)

Follow `docs/superpowers/runbooks/postgres-foundation-cutover.md` in exact sequence:

1. **Pre-Cutover Channel Verification (Staging / Non-Production):**
   - **Discord:** Confirm bot application has Gateway Message Content intents enabled; verify `/todo`, `/expense`, `/portfolio`, `/ask`, and `/link` in DM and guild.
   - **Telegram:** Verify `/start`, `/todo` inline keyboard buttons ("Done #id"), `/link`, and secret header verification (`TELEGRAM_WEBHOOK_SECRET`) in webhook mode.
   - **Email:** Verify IMAP mailbox polling over TLS, DMARC authentication check (`EMAIL_AUTHSERV_ID`), and multipart plain/HTML reply formatting via SMTP.

2. **Production Database Pre-Flight & Migration Rehearsal:**
   - **Drain Legacy Webhooks:** Check `SELECT COUNT(*) FROM webhook_outbox WHERE status = 'pending'`. Allow the legacy binary to drain pending alerts, or accept dropping them (migration `0005` drops `webhook_outbox`).
   - **Check Orphan Matrix Links:** Verify `SELECT l.mxid FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id WHERE a.id IS NULL;` returns 0 rows. Remove any orphaned links before migration `0007` backfills to `platform_identities`.
   - **Rehearse Migrations on Production Dump:**
     ```bash
     pg_dump -Fc "$PROD_URL" > prod.dump
     createdb rehearsal
     pg_restore -d rehearsal --no-owner prod.dump
     MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
     PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
       cargo test -p matrix-bot migrates_a_real_database -- --ignored
     ```

3. **Supervised Production First Boot:**
   - Take a complete backup of PostgreSQL and the `data/` directory (`bot_store/`, `matrix_session.json`, `matrix_token.json`, `verified_users.json`).
   - Ensure writable storage directory (`STORAGE_LOCAL_ROOT`, default `data/storage`).
   - Boot binary and verify log output:
     - Log confirms restored session: `Reusing stored Matrix session for ... on device <id>`.
     - Log confirms cross-signing self-verification: `Device::verify()` succeeded (`Ok`).
     - Check bot account for stray device IDs from prior testing.
     - Send `!say Hello` in a Matrix room to verify MSC3245 audio voice bubble with waveform rendering.
     - Verify `SELECT platform, COUNT(*) FROM platform_identities GROUP BY platform;` reflects all migrated user links.

---

### Track 2: Post-Cutover Enhancements & Technical Debt

1. **Deep Procedural Pipelines for Built-in Agent Tasks (`backend/src/domain/agent/task.rs`):**
   - The 5 standard task plugins (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`) currently formulate goals executed through the agent's native tool calling.
   - Expand specialized procedural workflows where appropriate:
     - Direct Kokoro TTS audio compilation for weekly bookmark digests.
     - Deterministic mathematical portfolio drift calculation before invoking LLM synthesis.

2. **Discord Channel Capabilities Expansion:**
   - Add interactive ActionRow buttons for task completion (matching Telegram's inline keyboard "Done" buttons).
   - Add typed slash command options (in addition to the single `args` string fast-path).
   - Inbound receipt/image attachment ingestion over Discord DMs (saving to `StorageService` and enqueuing `media.process_receipt`).

3. **WebAuthn Conditional UI (Passkey Autofill):**
   - Implement passkey autofill on the web sign-in form using the WebAuthn Conditional UI standard (`mediation: 'conditional'`) for seamless page-load sign-in.

4. **Integration Test for Portfolio Price Refresh Snapshot:**
   - Add an integration test asserting that `RefreshPricesHandler` persists today's USD snapshot row in `portfolio_daily_snapshots` (currently tested at the service layer in `application/portfolio/mod.rs`).

5. **Minor Deferred Edge Cases:**
   - `SyncHealth`: Add fallback handling if Matrix sync timestamp string fails to parse (theoretical).
   - Morning briefing window: Wrap-around logic for target briefing windows close to the hour boundary (`:58`–`:02`).

---

## 5. Consolidated Architectural Invariants

- **PostgreSQL 18 Exclusivity:** The application database is PostgreSQL only. `BIGINT` for row IDs, native `UUID` for `accounts.id` and `owner_id`. No SQL in `domain/`, `application/`, `jobs/`, or `interface/`.
- **Transactional Outbox:** State mutations and their corresponding `SystemEvent` must be committed in the same database transaction. Outbox deliveries handle external fan-out with independent retries.
- **Unified Tool Registry:** Interactive user chat uses direct LLM + tool calling bounded by `MAX_TOOL_ROUNDS = 5`. The autonomous agent loop (native tool calling, one tool call per step) is strictly reserved for scheduled background workflows.
- **pgvector Hybrid Search:** Multi-chunk embeddings (`vector(384)`) indexed via HNSW (`vector_cosine_ops`), populated asynchronously via transactional outbox + background durable jobs (`knowledge.embed`), combining vector cosine distance with full-text search without external vector DBs.
- **Account Identity Model:** One human account (`accounts.id`) links multiple chat identities across Matrix, Discord, Telegram, and Email via `platform_identities`.
- **File Length & Code Quality:** No file in `backend/src/` or `inference/src/` may exceed 500 lines of production code (target < 300 lines). Every feature follows:
  `domain/<f>` (pure rules) → `application/<f>` (service & transactions) → `infrastructure/postgres/repositories/<f>.rs` (SQL) → `interface/` (DTOs & chat handlers).

---

## 6. Verification Suite Reference

Run these commands before any commit:

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p matrix-bot --all-targets
cargo test -p matrix-bot --test architecture
cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run build
npm --prefix frontend run test:e2e
```
