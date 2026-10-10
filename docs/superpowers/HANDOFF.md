# Agent Handoff — Production Cutover & Remaining Roadmap

**Read this before touching code.** This document is the authoritative hand-off state for `matrix-bot`. It outlines the current state of the platform, immediate operational cutover actions, remaining roadmap enhancements, and settled architectural invariants.

> **Current Status (2026-10):** All architectural phases across the PostgreSQL foundation, omnichannel adapters, storage decoupling, autonomous agent runtime, web console polish, **Notes & Knowledge Base (pgvector + hybrid search)**, and **Dynamic Configuration & Encrypted Credential Vault** are **implemented and verified**.

---

## 1. Quick Start & Architecture Overview

`matrix-bot` is an **Omnichannel Personal AI/Life Workspace & Knowledge Base** built around Matrix, Discord, Telegram, Email, and a React 19 Web Console, powered by a modular Rust backend and local ONNX inference server.

- `backend/` (`matrix-bot`): Matrix bot, Axum REST/SSE API, MCP server, LLM gateway, durable jobs, transactional outbox, knowledge base (pgvector), dynamic configuration & AES-256-GCM secrets vault, and domain capabilities.
- `inference/` (`matrix-bot-inference`): Axum server hosting local ONNX models for OCR, speech-to-text (Sherpa/SenseVoice), text-to-speech (Kokoro-82M), and text embeddings (`multilingual-e5-small`).
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
infrastructure/crypto/       AES-256-GCM secret encryption with AAD binding and fail-closed fallback
infrastructure/storage/      StorageService: Local filesystem (HMAC presigning) and S3/R2
infrastructure/inference/    HTTP clients: OCR, STT, TTS, embeddings, LLM transport, provider testers
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
    - Step-up verification (`VerifiedUser`, 5-minute window `STEP_UP_WINDOW_MINUTES = 5`) is strictly enforced for sensitive endpoints (`/account/*`, `/portfolio/trade`, `/agent/approvals/*`). Ordinary admin `GET` endpoints use `AdminSession` (`role == 'admin'`, enrolled passkey required, non-MCP token, non-recovery-restricted session), while admin write and secret-test endpoints (`PUT`, `POST`, `DELETE`) require `AdminUser` (`AdminSession` + 5-minute passkey step-up).
    - Initial administrator bootstrap is CLI/SQL-only (`matrix-bot admin grant <account_id>` / `matrix-bot admin revoke <account_id>`); public registration and passkey enrollment never grant `admin` implicitly.
    - Secret read APIs return metadata only (`SecretMetadata`); plaintext and ciphertext never appear in response DTOs, DOM, logs, or audit records.
    - Sensitive commands are strictly refused in public rooms/channels; `/link` is DM-only with 10-minute single-use tokens.
    - Every owner-keyed table must be registered in `OWNED_TABLES` or `ACCOUNT_ID_FOLLOWS` (`domain/accounts/mod.rs`).
    - Every user-supplied string rendered into HTML must pass through `html_escape()`; input strings are capped at 500 characters.

---

## 3. Verified System Baseline

- **Rust Backend:**
  - `cargo check -p matrix-bot --all-targets`: Clean (zero errors, zero warnings).
  - `cargo clippy --workspace --all-targets -- -D warnings`: Clean.
  - `cargo test -p matrix-bot --lib`: 1583 passed / 0 failed.
  - `cargo test -p matrix-bot --test architecture`: 39 passed / 0 failed.
  - `cargo test -p matrix-bot live_postgres`: 3 passed / 0 failed.
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: Byte-identical (zero drift).
- **Inference Server:**
  - `cargo test -p matrix-bot-inference`: 94 passed / 0 failed.
- **Frontend SPA:**
  - `npm --prefix frontend run build`: Clean (1779 modules transformed, zero TypeScript errors).
  - `npm --prefix frontend run test:e2e`: 223 passed / 0 failed (100% green across all Playwright specs).

---

## 4. Remaining Tasks & Action Plan

### Track 1: Production Cutover & Operator Steps (Human Operator / User-Gated)

Follow `docs/superpowers/runbooks/postgres-foundation-cutover.md` in sequence:

1. **Pre-Cutover Channel Verification (Staging / Non-Production):**
   - **Discord:** Confirm bot application has Gateway Message Content intents enabled; verify `/todo`, `/expense`, `/portfolio`, `/ask`, and `/link` in DM and guild.
   - **Telegram:** Verify `/start`, `/todo` inline keyboard buttons ("Done #id"), `/link`, and secret header verification (`TELEGRAM_WEBHOOK_SECRET`) in webhook mode.
   - **Email:** Verify IMAP mailbox polling over TLS, DMARC authentication check (`EMAIL_AUTHSERV_ID`), and multipart plain/HTML reply formatting via SMTP.

2. **Production Database Pre-Flight & Migration Rehearsal:**
   - **Drain Legacy Webhooks:** Check `SELECT COUNT(*) FROM webhook_outbox WHERE status = 'pending'`. Allow the legacy binary to drain pending alerts, or accept dropping them (migration `0005` drops `webhook_outbox`).
   - **Check Orphan Matrix Links:** Verify `SELECT l.mxid FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id WHERE a.id IS NULL;` returns 0 rows. Remove any orphaned links before migration `0007` backfills to `platform_identities`.
   - **Legacy Notes Derived-Column Check (if upgrading a database with pre-`0013` notes):** Migration `0014` makes `content_hash`, `search_text`, and `excerpt` `NOT NULL`. If upgrading an environment with existing notes from `0011`, deploy commit `0f01833` first, let `knowledge.backfill_embeddings` populate those columns, verify `SELECT count(*) FROM notes WHERE content_hash IS NULL OR search_text IS NULL OR excerpt IS NULL;` returns `0`, then deploy the current release. Fresh databases or environments without legacy notes upgrade directly.
   - **Rehearse Migrations on Production Dump:**
     ```bash
     pg_dump -Fc "$PROD_URL" > prod.dump
     createdb rehearsal
     pg_restore -d rehearsal --no-owner prod.dump
     MIGRATE_TEST_DATABASE_URL=postgres://postgres:bot@localhost:5432/rehearsal \
     PG_TEST_URL=postgres://postgres:bot@localhost:5432/postgres \
       cargo test -p matrix-bot migrates_a_real_database -- --ignored
     ```

3. **Inference & Secrets Environment Setup:**
   - Deploy `matrix-bot-inference` and wait until `/health` is ready (first start downloads the ~135 MB `multilingual-e5-small` model into `EMBEDDING_MODEL_DIR` and verifies its SHA-256).
   - Set `EMBEDDING_MODEL=multilingual-e5-small` and configure a 32-byte `SECRETS_ENCRYPTION_KEY` (`openssl rand -base64 32`) in production before booting the backend.

4. **Supervised Production First Boot:**
   - Take a complete backup of PostgreSQL and the `data/` directory (`bot_store/`, `matrix_session.json`, `matrix_token.json`, `verified_users.json`).
   - Ensure writable storage directory (`STORAGE_LOCAL_DIR`, default `data/storage`).
   - Boot binary and verify log output:
     - Log confirms restored session: `Reusing stored Matrix session for ... on device <id>`.
     - Log confirms cross-signing self-verification: `Device::verify()` succeeded (`Ok`).
     - Check bot account for stray device IDs from prior testing.
     - Send `!say Hello` in a Matrix room to verify MSC3245 audio voice bubble with waveform rendering.
     - Verify `SELECT platform, COUNT(*) FROM platform_identities GROUP BY platform;` reflects all migrated user links.
     - Grant initial administrator role to your passkey-enrolled account via `matrix-bot admin grant <account_id>`.

---

### Track 2: Post-Cutover Enhancements & Technical Debt

1. **Wire Scheduled Agent Task Tool Whitelist (`backend/src/interface/jobs/agent.rs`):**
   - `ScheduledTaskDefinition::allowed_tools()` is defined in the domain, but the job handler currently passes `allowed_tools: None` to `evaluate_step` and `execute_approved`. Wire the task's tool whitelist through the job execution context.

2. **Quote-Based AI Confirmation Without Taint:**
   - Every quoted message currently taints the AI turn (including quotes of the bot's own messages, since webhook alerts and notices also come from the bot account). Users confirming a sensitive action in clients that auto-quote replies must send an unquoted message. To allow quote-based confirmation safely, track authentic AI reply event IDs (in-memory bounded set or an `ai_reply_messages` table).

3. **Deep Procedural Pipelines for Built-in Agent Tasks (`backend/src/domain/agent/task.rs`):**
   - Expand specialized procedural workflows for the 5 built-in task plugins (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`):
     - Direct Kokoro TTS audio compilation for weekly bookmark digests.
     - Deterministic mathematical portfolio drift calculation before invoking LLM synthesis.

4. **Discord Channel Capabilities Expansion:**
   - Add interactive ActionRow buttons for task completion (matching Telegram's inline keyboard "Done" buttons).
   - Add typed slash command options (in addition to the single `args` string fast-path).
   - Inbound receipt/image attachment ingestion over Discord DMs (saving to `StorageService` and enqueuing `media.process_receipt`).

5. **WebAuthn Conditional UI (Passkey Autofill):**
   - Implement passkey autofill on the web sign-in form using the WebAuthn Conditional UI standard (`mediation: 'conditional'`) for seamless page-load sign-in.

6. **Integration Test for Portfolio Price Refresh Snapshot:**
   - Add an integration test asserting that `RefreshPricesHandler` persists today's USD snapshot row in `portfolio_daily_snapshots` (currently tested at the service layer in `application/portfolio/mod.rs`).

7. **Minor Deferred Edge Cases:**
   - `SyncHealth`: Add fallback handling if Matrix sync timestamp string fails to parse.
   - Morning briefing window: Wrap-around logic for target briefing windows close to the hour boundary (`:58`–`:02`).

---

## 5. Consolidated Architectural Invariants

- **PostgreSQL 18 Exclusivity:** The application database is PostgreSQL only. `BIGINT` for row IDs, native `UUID` for `accounts.id` and `owner_id`. No SQL in `domain/`, `application/`, `jobs/`, `events/`, or `interface/`.
- **Transactional Outbox:** State mutations and their corresponding `SystemEvent` must be committed in the same database transaction. Outbox deliveries handle external fan-out with independent retries.
- **Unified Tool Registry & Untrusted Input Guard:** Interactive user chat uses direct LLM + tool calling bounded by `MAX_TOOL_ROUNDS = 5`. Outside content (`<quoted_message>`, `<ocr_text>`, `<tool_output>`) escapes `<` and taints the turn, requiring explicit user confirmation before sensitive tools execute. The autonomous agent loop uses native tool calling (one tool call per step) and is reserved for background workflows, executing approved sensitive actions at-most-once from stored calls.
- **pgvector Hybrid Search:** Multi-chunk `multilingual-e5-small` embeddings (`vector(384)`, served by `matrix-bot-inference`, tagged with the model that made them) indexed via HNSW (`vector_cosine_ops`), populated asynchronously via transactional outbox + background durable jobs (`knowledge.embed`). Search fuses lexical (FTS + trigram/substring) and vector results via reciprocal rank fusion, degrading to lexical-only if the embedder is unavailable.
- **Dynamic Configuration & Encrypted Vault:** `RuntimeConfigSnapshot` (`ArcSwap`) provides lock-free reads backed by `app_settings` and `SettingRegistry` defaults (20 hot-reloadable settings, all wired to live runtime consumers; restart-required settings stay in `.env`), reloaded on `ConfigUpdated`/`SecretChanged` outbox notifications. External credentials in `provider_secrets` use AES-256-GCM with 96-bit CSPRNG nonces and AAD binding (`{provider}:{name}:{key_version}`), failing closed when `SECRETS_ENCRYPTION_KEY` is absent. Precedence: `AiUserConfig > Runtime Snapshot > Environment > Defaults`.
- **Rulings (2026-10-10):**
  - *Ruling: CLI/SQL-only admin bootstrap (`matrix-bot admin grant|revoke <account_id>`) — eliminates first-user auto-promotion race conditions and requires an enrolled passkey before granting or using admin privileges — cost if wrong: operator runs one CLI command after first passkey enrollment.*
  - *Ruling: `AdminSession` for admin `GET`s and `AdminUser` (5-minute step-up window) for admin writes and secret connectivity tests — prevents unnecessary WebAuthn prompts when browsing the Admin Console while gating all mutations and secret operations — cost if wrong: none, secret `GET` returns masked metadata only.*
  - *Ruling: Keep only the 20 hot-reloadable settings in `SettingRegistry` and the 6 hot-reloadable secret providers in `SecretProvider` (`OpenAi`, `Anthropic`, `Tavily`, `Stt`, `Tts`, `Generic`), keeping restart-required settings and credentials (channels, `EMBEDDING_MODEL`, and S3) in `.env` — avoids misleading Admin UI settings/secrets that cannot hot-reload and keeps `domain/config/registry.rs` under the 500-line limit — cost if wrong: changing channel adapters, S3, or embedding model requires `.env` edit + restart, which is already required by their lifecycle.*
- **Account Identity Model:** One human account (`accounts.id`) links multiple chat identities across Matrix, Discord, Telegram, and Email via `platform_identities`. Instance-wide tables (`app_settings`, `provider_secrets`, `config_audit_log`) are global and registered in `AUDIT_ACTOR_UNTOUCHED`.
- **File Length & Code Quality:** No file in `backend/src/` or `inference/src/` may exceed 500 lines of production code (target < 300 lines). Only `config/` reads environment variables.

---

## 6. Verification Suite Reference

Note for fresh local test PostgreSQL servers: create `vector` and `pg_trgm` extensions once in `public` (`CREATE EXTENSION IF NOT EXISTS vector SCHEMA public; CREATE EXTENSION IF NOT EXISTS pg_trgm SCHEMA public;`) before running parallel tests.

Run these commands before any commit:

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p matrix-bot --lib -- --test-threads=4
cargo test -p matrix-bot --test architecture
cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json
npm --prefix frontend run build
npm --prefix frontend run test:e2e
```
