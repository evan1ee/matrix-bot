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
   - **Drain Legacy Webhooks:** Check `SELECT COUNT(*) FROM webhook_outbox WHERE status = 'pending'`. Allow the legacy binary to drain pending alerts, or accept dropping them (migration `0004` drops `webhook_outbox`).
   - **Check Orphan Matrix Links:** Verify `SELECT l.mxid FROM matrix_links l LEFT JOIN accounts a ON a.id = l.account_id WHERE a.id IS NULL;` returns 0 rows. Remove any orphaned links before migration `0006` backfills to `platform_identities`.
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

1. **Quote-Based AI Confirmation Without Taint (Fixed):**
   - Implemented authentic AI reply tracking via in-memory bounded cache (`AiReplyTracker`) in `LlmService`. Sent AI response message IDs are recorded by channel adapters (Matrix `show_reply` capturing both placeholder and replacement event IDs, and Telegram worker). Inbound quotes are verified against `(room_key, event_id, owner_id)`. Quoting genuine AI confirmation prompts avoids turn taint and allows held action execution without infinite confirmation loops, while webhook alerts, foreign messages, and cross-room/cross-owner quotes strictly remain tainted.

2. **Deep Procedural Pipelines for Built-in Agent Tasks (Fixed):**
   - Implemented 4 deterministic domain calculation engines with integer minor-unit math and strict invariant validation:
     - `portfolio::rebalance::evaluate_portfolio_drift`: Computes asset allocation drift percentages against target weights and derives rebalancing actions (`Buy`, `Sell`, `InBand`) without direct trade execution.
     - `finance::audit::evaluate_monthly_audit`: Aggregates transactions, splits recurring bills vs discretionary spending, computes percentage distributions, and flags category spending spikes exceeding a configurable threshold.
     - `recurring_bills::due::evaluate_bill_cash_adequacy`: Groups recurring bills by temporal urgency buckets (`DueToday`, `DueWithin3Days`, `DueWithin14Days`, `Later`) and evaluates cash reserve adequacy against liquid cash account balances.
     - `habits::consistency::evaluate_habit_consistency`: Evaluates 7-day completion rates, tracks active streaks, and categorizes trends (`Strong`, `Stable`, `AtRisk`, `Slipping`).
   - Exposed 4 analytical tools to the autonomous agent and chat LLM via `llm/tools/` conforming to the OpenAI schema standard (`evaluate_portfolio_rebalance`, `audit_monthly_finances`, `evaluate_bill_due_adequacy`, `evaluate_habit_consistency`).
   - Enhanced all 5 built-in task plugins (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`) with specialized tool whitelists, deep procedural prompts, and configurable thresholds.
   - Built multi-modal Kokoro TTS audio compilation pipeline in `backend/src/interface/jobs/agent/audio.rs` and `mod.rs`: extracts `<audio_script>...</audio_script>`, synthesizes Ogg Opus audio via `TtsClient`, attaches it to delivered messages (`AttachmentKind::Audio`), cleanly strips tags from plain/HTML text, and degrades fail-open if TTS is unconfigured or unavailable.

3. **Discord Channel Capabilities Expansion (Fixed):**
   - Implemented interactive ActionRow buttons in `render.rs` and `outbound.rs` chunked up to 5x5 per message (`CreateActionRow::Buttons`), with component interaction parsing in `inbound.rs` and dispatching in `gateway.rs` and `worker.rs` (`Work::Component`).
   - Implemented typed slash command options and subcommands in `commands.rs` and `inbound.rs` (`/todo add task:`, `/todo done id:`, `/todo list page:`, `/expense add amount: note:`, `/expense list`) with backward-compatible single `args` string parsing.
   - Implemented inbound receipt/image attachment ingestion over Discord DMs in `gateway.rs` and `worker.rs` (`Work::DirectAttachment`), resolving linked account owners, enforcing 10 MiB limit and download timeout, persisting into object storage (`StorageCategory::Receipts`), enqueuing durable `media.process_receipt` background jobs, and replying with receipt acknowledgments.

4. **WebAuthn Conditional UI (Passkey Autofill) (Fixed):**
   - Added `isConditionalMediationAvailable()` check and `opts?: SignInOptions` (`{ conditional, signal }`) in `frontend/src/core/passkey.ts`.
   - Integrated passkey autofill input (`autoComplete="username webauthn"`) in `frontend/src/components/AuthModal.tsx` in `'signin'` mode.
   - Managed silent background conditional mediation lifecycle with `AbortController`, cleanly aborting on modal close, mode transitions, and manual sign-in without user-facing alert popups.
   - Added comprehensive Playwright E2E virtual authenticator tests verifying autofill, cancellation, and manual fallback.

5. **Minor Deferred Edge Cases (Fixed):**
   - `SyncHealth`: Fallback handling added (`|| 'unknown'`) when formatting last sync timestamp in web UI.
   - Morning briefing window: Implemented wrap-around logic for target briefing windows across hour boundaries (`:58`–`:02`) and across midnight.

6. **Review follow-ups left open by the tech-debt pass (Fixed):**
   - `ScheduledAgentTask::allowed_tools()` defaults to `Some(Vec::new())` (fail-closed) and asserted `is_some()` in `every_builtin_whitelist_names_registered_tools`.
   - `resume.rs` checks `tool_policy` before `begin_approved_execution` commits the "started" marker.
   - Agent execution job handler, `chat/llm.rs` (`handle_show_config`), and `http/ai.rs` (`get_ai_config`) resolve via `ResolvedAiConfig::resolve_effective(...)` honoring the dynamic snapshot and credential vault.
   - `grant_admin`/`revoke_admin` audit the CLI/system operator as actor (`None`); account merges that carry over the admin role record an `AdminGranted` audit entry with `reason: account_merge`.
   - `SearchService` caches and reuses the dynamic embedding HTTP client while the custom base URL and model remain unchanged.
   - `resolve_runtime_stt` and `resolve_runtime_tts` log warnings on vault decrypt errors instead of silently dropping them.
   - `POST /agent/approvals/:id/approve` requires `VerifiedUser` (enforcing step-up within 5 minutes) and is registered in `SENSITIVE`.

---

## 5. Settled rulings

Format: `Ruling: <decision> — <why> — <cost if wrong>`. New rulings go here in the same commit as the work (AGENTS §4.1).

### Rulings (2026-10-10)

- *Ruling: CLI/SQL-only admin bootstrap (`matrix-bot admin grant|revoke <account_id>`) — eliminates first-user auto-promotion race conditions and requires an enrolled passkey before granting or using admin privileges — cost if wrong: operator runs one CLI command after first passkey enrollment.*
- *Ruling: `AdminSession` for admin `GET`s and `AdminUser` (5-minute step-up window) for admin writes and secret connectivity tests — prevents unnecessary WebAuthn prompts when browsing the Admin Console while gating all mutations and secret operations — cost if wrong: none, secret `GET` returns masked metadata only.*
- *Ruling: Keep only the 20 hot-reloadable settings in `SettingRegistry` and the 6 hot-reloadable secret providers in `SecretProvider` (`OpenAi`, `Anthropic`, `Tavily`, `Stt`, `Tts`, `Generic`), keeping restart-required settings and credentials (channels, `EMBEDDING_MODEL`, and S3) in `.env` — avoids misleading Admin UI settings/secrets that cannot hot-reload and keeps `domain/config/registry.rs` under the 500-line limit — cost if wrong: changing channel adapters, S3, or embedding model requires `.env` edit + restart, which is already required by their lifecycle.*

### Rulings (2026-10-12 — Migration squash)

- *Ruling: squashed migrations 0003–0016 into 0003–0011 (0005 folded into 0004; 0012–0014 into 0010 `knowledge_base`; 0016 into 0011 `configuration_and_secrets`; the rest renumbered) — nothing past `0001` had been released and the production cutover had not happened, so only dev/test databases had applied them, and the squashed files build the identical final schema — a database that already recorded the old versions in `_sqlx_migrations` fails with a checksum/missing-migration error and must be recreated; the notes legacy-derived-column runbook step and the 0014 guard are gone with it. After the first production cutover, committed migrations are immutable again.*

### Rulings (2026-10-12 — Adversarial Review Remediation)

- *Ruling: `AuthUser::never_presented_passkey()` (`last_verified_at <= UNIX_EPOCH`) decides which sessions `AdminSession` refuses — `create_unverified` stores the Unix epoch because `timestamptz` cannot hold `MIN_UTC`, so the old `== MIN_UTC` check let an admin's Matrix access-token session read admin endpoints — a future sentinel must stay at or below the epoch.*
- *Ruling: Passkey enrollment via `/api/v1/account/passkeys/register/*` requires `VerifiedUser` (5-minute step-up window) and rejects MCP/recovery-restricted sessions; recovery enrollment uses `/api/v1/auth/recovery/*` exclusively (`C2`), and Matrix login sessions carry `last_verified_at = NULL` (`H1`) — closes privilege escalation from unverified/MCP/recovery sessions and Matrix tokens — cost if wrong: enrolling an additional passkey from the web console prompts for existing passkey step-up.*
- *Ruling: Room/channel delivery targets (`authorize_delivery`, `H4`/`M12`/`M26`) allow only `web`, an account's cached Matrix room, or a conversation address (`discord:`, `telegram:`, `email:`) matching the account's linked `platform_identities`; synthetic origins (`mcp`, `agent:*`) resolve to the user's saved default room or `web` — prevents cross-user message injection via raw room IDs or foreign channel addresses — cost if wrong: rooms not yet in the user's room cache cannot be targeted by raw ID until synced.*
- *Ruling: All outbound HTTP clients for user-configurable or untrusted URLs (`llm/client.rs`, `web/metadata.rs`, `H2`/`H3`/`M14`) disable automatic redirect following or re-validate each redirect hop and pin validated DNS socket addresses via `reqwest::ClientBuilder::resolve_to_addrs` — prevents SSRF via open redirects, IPv4-mapped IPv6 (`::ffff:127.0.0.1`), and DNS rebinding — cost if wrong: none.*
- *Ruling: Minor-unit currency exponents (`domain::finance::money::exponent_for`, `H7`/`H8`/`H9`/`M5`/`M19`/`H11`/`H13`/`M23`) are enforced end-to-end across recurring bills, footprints, portfolio quotes/alerts, net worth (skipping unconvertible currencies rather than assuming 1.0 EUR), LLM tool inputs, and the SPA — eliminates 100× scaling bugs on zero-decimal (`JPY`) and three-decimal (`KWD`) currencies — cost if wrong: none.*

### Rulings (2026-10-13 — Optional Social Channels & Omnichannel Web Support)

- *Ruling: `MATRIX_USERNAME` and `MATRIX_PASSWORD` are optional at boot (`Config::matrix_enabled()`); `SyncHealth::mark_disabled()` marks `/health/ready`'s `matrix_sync` check vacuous (`ok: true`, `last_sync_at: None`) when Matrix is unconfigured — allows the server to run with 0, 1, or any combination of social channels (`Matrix`, `Telegram`, `Discord`, `Email`, or Web-only) without altering `ReadyResponse` in `openapi.json` — cost if wrong: partial Matrix config (`MATRIX_USERNAME` without `MATRIX_PASSWORD`) still fails closed in `Config::validate()`.*
- *Ruling: `RoomService::default_room`, `RoomService::resolve_target_room`, `GET /api/v1/rooms`, and job delivery handlers filter destinations by `Config::configured_channels()` (`Outbound::is_configured`) so linked identities on disabled channels fall back to `web` instead of deferring forever; `CheckAlertsHandler` checks per-owner readiness (at least one linked identity on a connected channel, or no linked identities on any configured channel) before marking alerts triggered; and browser notifications (`Notification` API) request permission only from user clicks in Settings and deduplicate across tabs via `tag` — ensures non-Matrix and Web-only accounts receive reminders, morning briefings, and price alerts without losing alerts during partial channel outages — cost if wrong: none, unowned or unconfigured cross-channel addresses resolve to `web`.*

### Rulings (2026-10-15 — Tech-Debt & Security/Config Hardening)

- *Ruling: Agent approval endpoint `POST /api/v1/agent/approvals/:id/approve` requires `VerifiedUser` and is registered in `SENSITIVE` — aligns the backend with the frontend's `useSensitiveAction` so sensitive AI actions cannot be approved by stale sessions or MCP tokens without a fresh passkey within 5 minutes — cost if wrong: approving an action prompts for passkey verification if the session was verified over 5 minutes ago.*
- *Ruling: `ScheduledAgentTask::allowed_tools()` defaults to `Some(Vec::new())` (fail-closed) — ensures custom or incomplete task plugins cannot execute unapproved tools in unattended background jobs, and `resume.rs` evaluates tool authorization before committing the started execution marker — cost if wrong: custom tasks must explicitly implement `allowed_tools` if they require tool access.*
- *Ruling: `ResolvedAiConfig::resolve_effective(...)` is the shared runtime entry point across chat, agent jobs, and API — ensures autonomous agents and UI reflect the admin console's active provider, model, and encrypted vault secrets rather than falling back strictly to environment variables — cost if wrong: none.*

### Rulings (2026-10-17 — Quote-Based AI Confirmation Hardening)

- *Ruling: `AiReplyTracker` verifies authentic AI replies via `(room_key, event_id, owner_id)` tuple and 1-hour TTL — guarantees quoting the bot's own AI prompts does not trigger untrusted input taint or trap users in infinite confirmation loops, while preventing cross-room replay, cross-owner privilege escalation, and untracked webhook alert steering — cost if wrong: quotes from restarts after > 1 hour fall back to requiring unquoted confirmation.*

### Rulings (2026-10-17 — Discord Channel Capabilities Expansion)

- *Ruling: Discord slash commands register subcommands (`/todo add`, `/todo done`, etc.) without a root-level `args` string option — Discord's interaction API strictly forbids combining root-level arguments and subcommands on the same command (HTTP 400 `50035`), while inbound parsing maintains backward-compatible `args` fallback — cost if wrong: users typing `/todo` without subcommands see Discord autocomplete guide them to subcommands.*
- *Ruling: Direct image ingestion over Discord is restricted strictly to direct messages (DMs) from linked accounts — attachments in guild channels or from unlinked senders fail closed with immediate return or `UNLINKED_REPLY` without downloading bytes or persisting storage — cost if wrong: guild members must DM the bot to scan receipts.*

### Rulings (2026-10-18 — Deep Procedural Agent Pipelines)

- *Ruling: Deterministic domain calculation engines (`portfolio::rebalance`, `finance::audit`, `recurring_bills::due`, `habits::consistency`) perform exact integer minor-unit math and rule evaluation prior to or during LLM synthesis — prevents floating-point inaccuracies, LLM hallucination in financial/streak math, and guarantees consistent analysis across all 5 built-in agent tasks — cost if wrong: none.*
- *Ruling: Multi-modal Kokoro TTS audio digest compilation extracts `<audio_script>...</audio_script>`, synthesizes Ogg Opus audio via `TtsClient`, attaches it as `AttachmentKind::Audio` to the delivered message, and strips the tag from plain/HTML text with fail-open graceful degradation — gives users audio podcast briefings in supported channels without breaking text delivery or aborting agent runs if TTS is unconfigured or unavailable — cost if wrong: none.*
- *Ruling: Non-execution trade safety invariant enforced across portfolio tools — `evaluate_portfolio_rebalance` calculates mathematical drift and recommended buy/sell rebalancing amounts, explicitly instructing the agent to explain adjustments without executing trades directly — prevents autonomous agent execution of destructive financial transactions without human-in-the-loop approval — cost if wrong: none.*

---

## 6. Verification

The verification gates are listed once, in [AGENTS.md §8](../../AGENTS.md#8-verification). Run them from the repository root.

A fresh local test PostgreSQL server needs the `vector` and `pg_trgm` extensions created once in `public` before parallel tests run: `CREATE EXTENSION IF NOT EXISTS vector SCHEMA public; CREATE EXTENSION IF NOT EXISTS pg_trgm SCHEMA public;`. Database setup is in [`docs/database.md`](../database.md#test-databases).
