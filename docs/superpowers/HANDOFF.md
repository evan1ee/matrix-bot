# Agent handoff

**Read this before touching code.** [`AGENTS.md`](../../AGENTS.md) holds the rules; this file holds the state: what is done, what an operator must still do, what is open, and the rulings that settled each disagreement. Design rationale lives in [`docs/architecture.md`](../architecture.md) and its sibling documents, not here.

> **Status (2026-10):** every architectural phase is implemented: the PostgreSQL foundation, omnichannel adapters, object storage, the autonomous agent runtime, the web console, the notes and knowledge base (pgvector hybrid search), and dynamic configuration with the encrypted credential vault. What remains is operator cutover (Track 1) and the follow-ups in Track 2.

---

## 1. Orientation

`matrix-bot` is a personal AI/life workspace reached through Matrix, Discord, Telegram, Email, a REST/SSE API, MCP and a React SPA. Crates and layers are described in [`AGENTS.md`](../../AGENTS.md) §1 and §3; the designs behind them in [`docs/architecture.md`](../architecture.md), [`database.md`](../database.md), [`api.md`](../api.md) and [`llm.md`](../llm.md).

## 2. Safety rules

The hard safety rules are [`AGENTS.md`](../../AGENTS.md) §9 and the security rules §7; they are not repeated here. In short: never touch the production `.env` or `data/`, never run the bot binary (only `cargo run --bin generate-openapi`), never push, merge, open a PR or deploy without an explicit instruction. Gitea (`git@git.mbm.one:evan/matrix-bot.git`) is primary and GitHub (`origin`) is a hand-updated mirror.

The step-up routes are whatever `SENSITIVE` lists in `interface/http/mod.rs::step_up_tests`; read the list, not a summary of it.

## 3. Verified baseline

[`reviews/2026-10-11-adversarial-review.md`](reviews/2026-10-11-adversarial-review.md): all 53 verified findings (`C1`–`C2`, `H1`–`H13`, `M1`–`M26`, `L1`–`L12`) across `matrix-bot`, `matrix-bot-inference` and the frontend are fixed (`Status: F`) with regression tests.

The gates in [`AGENTS.md`](../../AGENTS.md) §8 were green when those fixes landed (`6859962`, 2026-10-12). Counts go stale, so none are recorded here: rerun the gates before relying on this line.

---

## 4. Remaining work

### Track 1: Production cutover (operator, user-gated)

Follow `docs/superpowers/runbooks/postgres-foundation-cutover.md` in sequence:

1. **Pre-Cutover Channel Verification (Staging / Non-Production):**
   - **Discord:** Confirm the bot needs no privileged gateway intent (it uses `GUILDS` and `DIRECT_MESSAGES`); verify `/todo`, `/expense`, `/portfolio`, `/ask`, and `/link` in DM and guild.
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

### Track 2: Post-cutover enhancements and technical debt

- Tech-debt register: [reviews/2026-10-10-tech-debt-audit.md](reviews/2026-10-10-tech-debt-audit.md). Phases 0 and 1 are done (except TD-05 beyond the admin module). Phase 2 next: TD-10 (reqwest 0.12, own branch), TD-13.

1. **Quote-Based AI Confirmation Without Taint:**
   - Every quoted message currently taints the AI turn (including quotes of the bot's own messages, since webhook alerts and notices also come from the bot account). Users confirming a sensitive action in clients that auto-quote replies must send an unquoted message. To allow quote-based confirmation safely, track authentic AI reply event IDs (in-memory bounded set or an `ai_reply_messages` table).

2. **Deep Procedural Pipelines for Built-in Agent Tasks (`backend/src/domain/agent/task.rs`):**
   - Expand specialized procedural workflows for the 5 built-in task plugins (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`):
     - Direct Kokoro TTS audio compilation for weekly bookmark digests.
     - Deterministic mathematical portfolio drift calculation before invoking LLM synthesis.

3. **Discord Channel Capabilities Expansion:**
   - Add interactive ActionRow buttons for task completion (matching Telegram's inline keyboard "Done" buttons).
   - Add typed slash command options (in addition to the single `args` string fast-path).
   - Inbound receipt/image attachment ingestion over Discord DMs (saving to `StorageService` and enqueuing `media.process_receipt`).

4. **WebAuthn Conditional UI (Passkey Autofill):**
   - Implement passkey autofill on the web sign-in form using the WebAuthn Conditional UI standard (`mediation: 'conditional'`) for seamless page-load sign-in.

5. **Minor Deferred Edge Cases:**
   - `SyncHealth`: Add fallback handling if Matrix sync timestamp string fails to parse.
   - Morning briefing window: Wrap-around logic for target briefing windows close to the hour boundary (`:58`–`:02`).

6. **Review follow-ups left open by the tech-debt pass:**
   - A vault key is attached to the env endpoint when `llm.default_provider` is not customized, even if the key belongs to another vendor than the endpoint (`llm/resolved_config.rs`, `resolve`). Decide whether to gate it by the provider's canonical URL; the existing precedence test encodes the current behaviour.
   - The agent job, `chat/llm.rs` and `http/ai.rs` resolve with `from_user_config` (env only), so agent runs ignore the admin console's provider, model and vault key while chat honours them.
   - `ScheduledAgentTask::allowed_tools()` defaults to `None`, which means every tool; make the default `Some(vec![])` and assert `is_some()` in `every_builtin_whitelist_names_registered_tools`.
   - `resume.rs` checks the whitelist after `begin_approved_execution` has committed its "started" marker; check `tool_policy` first.
   - `grant_admin`/`revoke_admin` audit the target account as the actor; account merges that carry over the admin role write no audit row.
   - `application/knowledge/search.rs` builds a new embedding HTTP client per call while the base URL is customized; `resolve_runtime_stt` drops decrypt errors without a warning.
   - Agent approvals (`POST /agent/approvals/:id/approve|reject`, `interface/http/agent.rs`) take `AuthUser`, not `VerifiedUser`, and are not in `SENSITIVE`; earlier notes of this file claimed step-up for them. Decide whether approving a sensitive AI action needs a fresh passkey (and whether an MCP token may approve at all); if so, add `VerifiedUser` and the `SENSITIVE` entries together.

---

## 5. Settled rulings

Format: `Ruling: <decision> — <why> — <cost if wrong>`. New rulings go here in the same commit as the work (AGENTS §4.1).

### Rulings (2026-10-10)

- *Ruling: CLI/SQL-only admin bootstrap (`matrix-bot admin grant|revoke <account_id>`) — eliminates first-user auto-promotion race conditions and requires an enrolled passkey before granting or using admin privileges — cost if wrong: operator runs one CLI command after first passkey enrollment.*
- *Ruling: `AdminSession` for admin `GET`s and `AdminUser` (5-minute step-up window) for admin writes and secret connectivity tests — prevents unnecessary WebAuthn prompts when browsing the Admin Console while gating all mutations and secret operations — cost if wrong: none, secret `GET` returns masked metadata only.*
- *Ruling: Keep only the 20 hot-reloadable settings in `SettingRegistry` and the 6 hot-reloadable secret providers in `SecretProvider` (`OpenAi`, `Anthropic`, `Tavily`, `Stt`, `Tts`, `Generic`), keeping restart-required settings and credentials (channels, `EMBEDDING_MODEL`, and S3) in `.env` — avoids misleading Admin UI settings/secrets that cannot hot-reload and keeps `domain/config/registry.rs` under the 500-line limit — cost if wrong: changing channel adapters, S3, or embedding model requires `.env` edit + restart, which is already required by their lifecycle.*

### Rulings (2026-10-12 — Adversarial Review Remediation)

- *Ruling: Passkey enrollment via `/api/v1/account/passkeys/register/*` requires `VerifiedUser` (5-minute step-up window) and rejects MCP/recovery-restricted sessions; recovery enrollment uses `/api/v1/auth/recovery/*` exclusively (`C2`), and Matrix login sessions carry `last_verified_at = NULL` (`H1`) — closes privilege escalation from unverified/MCP/recovery sessions and Matrix tokens — cost if wrong: enrolling an additional passkey from the web console prompts for existing passkey step-up.*
- *Ruling: Room/channel delivery targets (`authorize_delivery`, `H4`/`M12`/`M26`) allow only `web`, an account's cached Matrix room, or a conversation address (`discord:`, `telegram:`, `email:`) matching the account's linked `platform_identities`; synthetic origins (`mcp`, `agent:*`) resolve to the user's saved default room or `web` — prevents cross-user message injection via raw room IDs or foreign channel addresses — cost if wrong: rooms not yet in the user's room cache cannot be targeted by raw ID until synced.*
- *Ruling: All outbound HTTP clients for user-configurable or untrusted URLs (`llm/client.rs`, `web/metadata.rs`, `H2`/`H3`/`M14`) disable automatic redirect following or re-validate each redirect hop and pin validated DNS socket addresses via `reqwest::ClientBuilder::resolve_to_addrs` — prevents SSRF via open redirects, IPv4-mapped IPv6 (`::ffff:127.0.0.1`), and DNS rebinding — cost if wrong: none.*
- *Ruling: Minor-unit currency exponents (`domain::finance::money::exponent_for`, `H7`/`H8`/`H9`/`M5`/`M19`/`H11`/`H13`/`M23`) are enforced end-to-end across recurring bills, footprints, portfolio quotes/alerts, net worth (skipping unconvertible currencies rather than assuming 1.0 EUR), LLM tool inputs, and the SPA — eliminates 100× scaling bugs on zero-decimal (`JPY`) and three-decimal (`KWD`) currencies — cost if wrong: none.*

---

## 6. Verification

The verification gates are listed once, in [AGENTS.md §8](../../AGENTS.md#8-verification). Run them from the repository root.

A fresh local test PostgreSQL server needs the `vector` and `pg_trgm` extensions created once in `public` before parallel tests run: `CREATE EXTENSION IF NOT EXISTS vector SCHEMA public; CREATE EXTENSION IF NOT EXISTS pg_trgm SCHEMA public;`. Database setup is in [`docs/database.md`](../database.md#test-databases).
