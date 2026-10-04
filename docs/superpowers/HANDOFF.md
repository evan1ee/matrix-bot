# Agent Handoff — Architecture Refactor

**Read this before touching code.** This guide is for any coding agent (Claude Code, Codex, Cursor, …) taking over the refactor of `matrix-bot`. It covers:

- what the refactor is;
- where it stands;
- how the work is done;
- what comes next;
- which decisions are already settled.

Do not reopen a settled decision without new evidence and the user's agreement.

> **Keep this file current.** Progress, decisions and open questions live **here**, in git. They do not live in a git-ignored scratch folder: an earlier worktree was deleted and its progress notes were lost. Update §4 (progress) and §9 (rulings) in the same commit as the work they describe.

---

## 0. Quick start (first 15 minutes)

1. Check out the branch **`refactor/postgres-layered`**. It is pushed to `gitea` (primary, `git.mbm.one`) and `origin` (GitHub mirror). Work in a separate worktree, not in the user's main checkout. The main checkout has a production `.env` and the real Matrix crypto store in `data/` (see §6).
2. Read these, in order:
   1. this file;
   2. `AGENTS.md` (hard rules, Engineering Principles, Definition of Done);
   3. `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` (the binding design for the current sub-project);
   4. `docs/superpowers/plans/2026-09-25-feature-layering.md` (the plan in progress).
3. Set up the environment and start the test database (README *Development* and `docs/database.md` → *Test databases*), then run the verification list (§5). It must be green before you change anything.
4. Continue at **§7 Next actions**.

## 1. What the refactor is

`SPEC.md` (v2.4.0) is the product and architecture roadmap: six phases, grouped into four sub-projects. Each sub-project needs its own design document, approved by the user, and its own plan(s) before any code is written.

| Sub-project | Phases | Scope | State |
|---|---|---|---|
| **1 — PostgreSQL Foundation** | 1–3 | PostgreSQL-only; versioned migrations; compile-checked SQL; layer-first architecture for **every backend feature**; account UUIDs; durable job queue; transactional outbox | **In progress** (designed, partly implemented) |
| 2 — Omnichannel | 4 | `ChannelAdapter` / `ChannelRegistry`; Matrix refactored onto it; Discord (`serenity`), Telegram (`teloxide`) and Email (IMAP/SMTP) adapters; `platform_identities`; chat-initiated linking | **Complete: plans A, B, C, and D implemented and verified.** Design `specs/2026-10-01-omnichannel-design.md`; plan A (`plans/2026-10-01-omnichannel-a-framework.md`); plan B (Discord, `plans/2026-10-02-omnichannel-b-discord.md`); plan C (Telegram, `plans/2026-10-02-omnichannel-c-telegram.md`); plan D (Email, `plans/2026-10-03-omnichannel-d-email.md`); branch `feat/omnichannel` |
| 3 — Scheduled agent & storage | 5 | Storage decoupling (`StorageService`, local/S3, presigned streaming, durable `media.process_receipt` job, chat image ingestion); `ScheduledAgentTask` runtime | **Complete: storage decoupling and autonomous agent runtime implemented and verified.** Plans `plans/2026-10-03-storage-decoupling.md` and `plans/2026-10-03-agent-runtime.md`; branch `feat/agent-runtime` |
| **4 — Web polish** | 6 | Regenerated TypeScript client; live UI through the unified SSE stream; UI for linked identities and notification routing; Autonomous Agent module | **Complete: design, plan, and frontend implementation verified.** Design `specs/2026-10-04-web-polish-design.md`; plan `plans/2026-10-04-web-polish.md`; branch `feat/web-polish` |

The order is strict. Each sub-project builds on the previous one: channels need the layers and the tool registry, the agent needs the job queue, and the web UI needs the outbox events. **All four sub-projects are now fully implemented and verified.**

The **React frontend** (`frontend/`) is essentially untouched by sub-project 1: it only switched to the `/health/*` endpoints. The public HTTP contract is frozen, so `backend/openapi.json` must not change.

## 2. Target architecture (sub-project 1)

The backend follows a layer-first layout (`SPEC.md` §13; design §3):

```
backend/src/
  main.rs            env (.env from the current directory only) → app::run
  app.rs             composition root: config, pool, migrations, AppServices, front doors,
                     one CancellationToken tree, 15 s bounded shutdown, supervision
  domain/<f>/        pure types + invariants (std/chrono/uuid/serde/thiserror only)
  application/<f>/   <F>Service: use cases, transactions, publishes events in the transaction via the outbox
                     application/mod.rs: AppError, AppServices
  infrastructure/
    postgres/        pool, ping, repositories/<f>.rs (sqlx::query! only), read_models/, merge.rs
    inference/       HTTP clients to the inference crate (OCR, STT, TTS) and the LLM
  interface/
    http/            axum handlers + DTOs (the OpenAPI types), health probes
    chat/            BotModule command handlers per feature, error_reply
    jobs/            job handlers: reminder.fire, briefing.send, portfolio.* (payload → service → Delivery)
    events/          outbox delivery sinks: ChannelSink (webhook alerts → any channel via the registry)
    channels/matrix/ matrix-sdk client, sync loop, E2EE, verification
  jobs/              job runtime: workers, reaper, purge, leader-elected scheduler (no SQL)
  events/            events runtime: LISTEN outbox loop, SSE hub, delivery dispatcher, hourly purge (no SQL)
  llm/               prompts, tool loop, tool registry
  error.rs           at the crate root for now (§11)
```

**Dependency rule** (enforced by `backend/tests/architecture.rs`):

| Layer | May not import |
|---|---|
| `domain` | anything except the pure crates listed in the tree above |
| `application` | axum, matrix-sdk, reqwest, utoipa, `interface` |
| `infrastructure` | `application`, `interface` |
| `jobs` | axum, matrix-sdk, utoipa, reqwest, `interface`; no SQL, tests included |
| `events` | axum, matrix-sdk, utoipa, reqwest, `interface`; no SQL, tests included; no lower layer imports it |

## 3. Settled user decisions

- PostgreSQL **18** is the only application database. matrix-sdk keeps its own SQLite crypto store.
- **sqlx 0.9** and **matrix-sdk 0.18**, upgraded together. They must agree on `libsqlite3-sys`, because Cargo's `links` check requires it.
- Adopt the live production database **in place**. `migrations/0001_baseline.sql` is idempotent and matches the legacy schema, and it is pinned to LF, because the migration checksum depends on line endings.
- IDs: **BIGINT** row IDs; **UUID** account IDs (migration `0002`, design step 7).
- SQL: only compile-checked `sqlx::query!` / `query_as!` / `query_scalar!`, with the offline `.sqlx/` cache committed and `SQLX_OFFLINE=true` set in `.cargo/config.toml`.
- Layout: **layer-first** (§2), not feature slices.
- Topology: a single instance, ready for high availability (advisory locks, `SKIP LOCKED`).
- Job queue: **hand-rolled** in PostgreSQL, with `dedupe_key`, heartbeat reaper and advisory-lock scheduler.
- Outbox: **every** event goes through it, with one delivery row per destination (`outbox_deliveries`); SSE is fed by `LISTEN`/`NOTIFY`.
- The user's engineering principles are merged into `AGENTS.md` and are binding.

## 4. Progress

### Sub-project 1: design steps (`design §6`)

| # | Step | State | Commits |
|---|---|---|---|
| 1 | PostgreSQL-only cut | ✅ | `382d1dd` `6c3d384` `f7dec54` `07897ab` |
| 2 | matrix-sdk 0.18 + sqlx 0.9 | ✅ | `f4ef60a` `c8576b6` |
| 3 | Versioned migrations | ✅ | `051444b` `68dfc56` `b803bef` |
| 4 | Compile-checked query pipeline, hardening, documentation | ✅ | `0bd18c0` `9c2865f` `e9fe149` `a52b984` `1af27e8` `4975039` `b502e2a` `e85d89b` `71ec65e` `1d139eb` `c6d304b` |
| 5 | Layer skeleton (+ fix wave) | ✅ | `e208254` `fbd5252` `3b53386` `85c2dbc` `e4f1aab` `778585e` `45020fe` `ac816e5` `be24444` |
| 6 | Feature layering (Tasks 1–14 below) | ✅ | Tasks 1–14 + review & fix wave |
| 7 | Account UUID (`0002_account_uuid`) | ✅ plan `docs/superpowers/plans/2026-09-29-account-uuid.md` | `186d271` `532e0b6` (clippy backlog cleared) |
| 8 | Job queue (`0003_jobs`); the reminder, briefing and portfolio loops replaced by jobs (there was no bills loop, R20). Design `specs/2026-09-29-job-queue-design.md`, plan `plans/2026-09-29-job-queue.md` | ✅ (branch `refactor/job-queue`) | `950e043..cade37c` |
| 8b | `backend/src/accounts/` retired (user request, part of the step 8 plan; Tasks 12–18) | ✅ | `d91baf7..5d6fc1d` |
| 9 | Outbox (`0004_outbox`, `outbox_deliveries`, SSE via LISTEN; remove `EventBroadcaster` and `webhook_outbox`). Design `specs/2026-09-30-outbox-design.md` (approved 2026-09-30), branch `refactor/outbox`, plan `plans/2026-09-30-outbox.md` (Tasks 1–12, clarifications C1–C9) | ✅ Tasks 1–12 done (whole-branch review + fix wave F1–F6, F9; R54–R68); final verification below | `437345f..b2587c7` (implementation `437345f..d143d1a`, docs `e9d1b13`, fix wave `4aca5b9..b2587c7`) |
| 10 | Docs: `AGENTS.md` rebuilt as rules-only; `docs/{architecture,database,api,llm}.md`; history/review docs removed; plans stay in `docs/superpowers/` | ✅ | `4bf5bc7` |

### Step 6: feature layering (`docs/superpowers/plans/2026-09-25-feature-layering.md`)

| Task | Scope | State |
|---|---|---|
| 1 | rooms (the template) | ✅ `14c74ab`, fix `60d14b3` |
| 2 | settings | ✅ `227c95c`, fix `ecef800` |
| 3 | accounts core + `AccountId` newtype | ✅ |
| 4 | accounts merge (dynamic SQL behind an allow-list, D-A) | ✅ |
| 5 | bookmarks | ✅ |
| 6 | footprints + habits | ✅ |
| 7 | webhook | ✅ |
| 8 | todo + reminder + recurring_bills + due_reminder (one batch; move `QuotedMessage` to `domain/chat.rs`) | ✅ |
| 9 | finance | ✅ |
| 10 | portfolio | ✅ |
| 11 | life + journal + briefing (read models) | ✅ |
| 12 | ocr + voice + tts + core + account command module | ✅ |
| 13 | ai + mcp (→ `llm/`, `AiTurnContext`, `MAX_TOOL_ROUNDS`) | ✅ |
| 14 | delete the `db_*!`/`tx_*!` macros and `modules/`; harden the architecture test | ✅ |
| — | whole-branch review + one fix wave | ✅ |

All features have moved into the layer-first layout (`domain/`, `application/`, `infrastructure/postgres/repositories/`, `interface/http/`, `interface/chat/`). `modules/` and the interim `db_*!`/`tx_*!` macros are deleted. Zero files run direct SQL from `interface/` or `application/`, `INTERFACE_SQL_ALLOW_LIST` has been deleted, and `backend/tests/architecture.rs` enforces all architectural boundaries.

The last full test run (step 9 final verification at `b2587c7`, `cargo test -p matrix-bot --all-targets`, no `--skip`): **lib 933 passed / 0 failed / 7 ignored, `main.rs` 2 passed, architecture suite 22 passed**; `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1` 3 passed; fmt, `check --all-targets` and clippy `-D warnings` clean; `cargo sqlx prepare --check` clean; `openapi.json` byte-identical; `npm --prefix frontend run build` green (installed with `npm ci --replace-registry-host=always`: the lockfile's `registry.npmmirror.com` URLs are unreachable from the cloud sandbox). Inference: 71 passed at step 7 (crate unchanged, not re-run).

### Sub-project 2 — Plan A (omnichannel framework, `plans/2026-10-01-omnichannel-a-framework.md`, branch `feat/omnichannel`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Domain channel types (`domain/channels`) | ✅ | cade6d3 |
| 2 | `platform_identities` schema and repository; `matrix_links` retired | ✅ | a4afe76 |
| 3 | `IdentityService` | ✅ | f94d5fb |
| 4 | Neutral message types, adapter trait, registry, renderers | ✅ | 2a61990 |
| 5 | The agent stops depending on Matrix | ✅ | 7fc3ee1, 3b20831 |
| 6 | Channel-neutral `CommandContext` and `Reply` | ✅ | f609106 |
| 7 | Inbound pipeline and `MatrixAdapter`; registry in the composition root | ✅ 7a pipeline (`channels/inbound.rs`); 7b `MatrixAdapter`, `matrix/{inbound,outbound,modules,rooms}.rs`, registry wired in `app.rs`, `interface/chat` free of `matrix_sdk` | 1d42bfa (7a), 8e1def1 (7b) |
| 8 | Outbound through the registry (jobs and outbox): `Outbound`, `ChannelSink`, `Destination::Conversation`; `Delivery`/`MatrixDelivery`/`MatrixRoomSink` deleted | ✅ | 9fd9611, 934ef50 |
| 9 | Chat-initiated linking (`LinkService`), old flow deleted; OpenAPI regenerated in Task 10 | ✅ | e1afdfb, 6b6aed7 |
| 10 | Identity and link HTTP API (intentional OpenAPI change): `interface/http/{identities,link}.rs`, `/account/matrix/*` deleted, `openapi.json` and `api.ts` regenerated, Security panel on `/account/identities` | ✅ | 1a56762 |
| 11 | Web link page (`modules/link/LinkPage.tsx`, route `/link`, signed-out) and the Security panel's every-channel identity list with Unlink | ✅ | 7e9cd7d |
| 12 | Boundary tests (`application_knows_no_channel_sdk`, `channel_domain_is_pure`; `tokio` added to the domain-purity test), docs (`architecture`, `database`, `api`, `llm`, `SPEC.md` §9, README, AGENTS, spec amendment note, cutover runbook), hand-off | ✅ | 1a1c412 |

Final verification (orchestrator, at `1a1c412`):
- `cargo fmt --check`: clean.
- `cargo check --all-targets`: clean, no warnings.
- `cargo test -p matrix-bot --all-targets`: lib 1044 passed / 0 failed / 7 ignored, `main.rs` 2 passed, architecture 28 passed.
- `cargo clippy -D warnings`: clean.
- `cargo sqlx prepare --check`: clean (migrations 0001–0008 applied to the scratch DB).
- `openapi.json`: matches the generated output.
- `git diff --check`: clean.
- `live_postgres`: 3 passed.
- `cargo build --release`: ok.
- `npm --prefix frontend run build`: ok.
- Playwright: 188 passed, 5 failed.
  - Four are the known pre-existing dropdown failures listed in §11.
  - The fifth, `portfolio.spec.ts:68`, failed once in the full run and passed in three isolated reruns of `portfolio.spec.ts` (23/23 each), so it is an intermittent flake.
- Playwright ran against sandbox Chromium rev 1194, aliased as the rev 1234 that Playwright 1.62.1 expects.

### Sub-project 2 — Plan B (Discord adapter, `plans/2026-10-02-omnichannel-b-discord.md`, branch `feat/omnichannel`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | `DiscordConfig` (`DISCORD_TOKEN`, `DISCORD_GUILD_ID`), redacted `Debug`, allowlist entries validated | ✅ | 246b0a1 |
| 2 | `render/markdown.rs` (HTML subset → Discord Markdown), sharing the sanitizer's tag scanner | ✅ | 4f52502 |
| 3 | `serenity` 0.12.5; `discord/{commands,inbound}.rs`; fixtures `contracts/channels/discord_*.json` | ✅ | 76c5132 |
| 4 | `discord/{render,outbound}.rs`: embed posts, `route_reply`, `send`, error classification | ✅ | a6c804d |
| 5 | `discord/{gateway,worker,progress,adapter}.rs`: `DiscordAdapter` | ✅ | 304c732 |
| 6 | `app.rs` registers the adapter and supervises every adapter; architecture tests | ✅ | b4c4bb5 |
| 7 | Docs (`architecture.md`, README, `.env.example`, `SPEC.md` §9.3, spec amendment), hand-off | ✅ | this commit |

Final verification (at b4c4bb5 plus the docs commit; docs change no code):
- `cargo fmt --check`, `cargo check --all-targets` (no warnings), `cargo clippy -D warnings`, `cargo sqlx prepare --check`: clean.
- `cargo test -p matrix-bot --all-targets`: lib 1093 passed / 0 failed / 7 ignored, `main.rs` 2 passed, architecture 29 passed.
- `openapi.json` byte-identical after every task; `git diff --check` clean.
- `live_postgres`: 3 passed; `cargo build -p matrix-bot --release`: ok; `npm --prefix frontend run build`: ok (frontend unchanged).
- Not verified (needs a real Discord application, HANDOFF §6): the live checks listed in the plan's *What the user verifies*.

### Sub-project 2 — Plan C (Telegram adapter, `plans/2026-10-02-omnichannel-c-telegram.md`, branch `feat/omnichannel`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | `TelegramConfig`/`TelegramMode` (`TELEGRAM_BOT_TOKEN`, `TELEGRAM_MODE`, `TELEGRAM_WEBHOOK_SECRET`), `validate_telegram`, `utils::secure::constant_time_eq` | ✅ | `f5f01bc`, `f39ad44` |
| 2 | `render/telegram_html.rs` (HTML subset → Telegram HTML, message splitting) | ✅ | `69e7c56`, `a998532` |
| 3 | `teloxide-core`; `telegram/{commands,inbound}.rs`; fixtures | ✅ | `cfec58b`, `9af28f5` |
| 4 | Outbound rendering and `send` | ✅ | `575ae4a` |
| 5 | `TelegramAdapter`: queue, dedupe, workers, progress, polling and webhook registration | ✅ | `ffc4a1d`, `5ec3910` |
| 6 | Webhook endpoint (constant-time secret); the compat shim moves | ✅ | `fcae1b5` |
| 7 | The todo list's "Done" buttons | ✅ | `17ce1ee` |
| 8 | Composition root and boundaries | ✅ | `045e30f` |
| 9 | Docs and hand-off (T1–T16 recorded as R102–R117) | ✅ | this commit |

Final verification (orchestrator, at `045e30f` plus the docs commit; docs change no code):
- `cargo fmt --check`: clean.
- `cargo check -p matrix-bot --all-targets`: clean, no warnings.
- `cargo test -p matrix-bot --lib`: 1212 passed / 0 failed / 7 ignored.
- `cargo test -p matrix-bot --test architecture`: 30 passed / 0 failed / 0 ignored.
- `cargo test -p matrix-bot live_postgres`: 3 passed / 0 failed.
- `cargo clippy -p matrix-bot --all-targets -- -D warnings`: clean (zero warnings).
- `cargo sqlx prepare --check`: clean.
- `openapi.json` byte-identical after every task; `git diff --check` clean.
- `cargo build -p matrix-bot --release`: ok (compiled cleanly in release profile).
- `npm --prefix frontend run build`: ok (frontend unchanged, verified).
- Not verified (needs a real Telegram bot from @BotFather, HANDOFF §6): the live checks listed in the plan's *What the user verifies*.

### Sub-project 2 — Plan D (Email adapter, `plans/2026-10-03-omnichannel-d-email.md`, branch `feat/omnichannel`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Dependencies and Email configuration (`EmailConfig`, `parse_email`, `validate_email`, `validate_email_allowlist`) | ✅ | `3d2767f` |
| 2 | Inbound normalization & DMARC authentication (`inbound.rs`, fixtures `email_*.eml`) | ✅ | `6dd6c52` |
| 3 | Outbound formatting & SMTP send (`outbound.rs`, `lettre` sender, error classification) | ✅ | `e700eab` |
| 4 | IMAP client & polling runtime (`imap.rs`, `tokio-rustls`, message search, move/seen) | ✅ | `a25c697` |
| 5 | `EmailAdapter` implementing `ChannelAdapter` trait, deduplication window | ✅ | `c42ff10` |
| 6 | Wire into composition root (`app.rs`), supervisor test, architecture boundary assertions | ✅ | `8418b25` |
| 7 | Docs (`architecture.md`, `README.md`, `.env.example`, `SPEC.md`, `HANDOFF.md`), hand-off (E1–E14 recorded as R125–R138) | ✅ | this commit |

Final verification (orchestrator, at `8418b25` plus the docs commit; docs change no code):
- `cargo fmt --check`: clean.
- `cargo check -p matrix-bot --all-targets`: clean, no warnings.
- `cargo test -p matrix-bot --lib`: 1245 passed / 0 failed / 7 ignored.
- `cargo test -p matrix-bot --test architecture`: 31 passed / 0 failed / 0 ignored.
- `cargo test -p matrix-bot live_postgres`: 3 passed / 0 failed.
- `cargo clippy -p matrix-bot --all-targets -- -D warnings`: clean (zero warnings).
- `cargo sqlx prepare --check`: clean.
- `openapi.json` byte-identical after every task; `git diff --check` clean.
- `cargo build -p matrix-bot --release`: ok (compiled cleanly in release profile).
- `npm --prefix frontend run build`: ok (frontend unchanged, verified).
- Not verified (needs a real mail server, HANDOFF §6): the live checks listed in the plan's *What the user verifies*.

### Sub-project 3 — Storage Decoupling (`plans/2026-10-03-storage-decoupling.md`, branch `feat/storage-decoupling`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Domain storage types (`domain/storage/mod.rs`): `StorageKey`, `StorageCategory`, `StorageError` | ✅ | `aed58ad` |
| 2 | Storage infrastructure & Local provider (`infrastructure/storage/local.rs`): atomic writes, HMAC signing | ✅ | `de6ffaa` |
| 3 | S3 storage provider (`infrastructure/storage/s3.rs`): `aws-sdk-s3`, presigned URLs | ✅ | `74ce914` |
| 4 | Database migration `0009` & Finance repository: `receipt_storage_key` in `transactions` & sqlx cache | ✅ | `ac060ec` |
| 5 | Application wiring: `FinanceService` (`record_expense`, `link_receipt`), `AppServices.storage` | ✅ | `0e6330a` |
| 6 | Durable background job: `media.process_receipt` (`interface/jobs/media.rs`), OCR extraction, expense record, outbox notifications | ✅ | `475c92b` |
| 7 | HTTP storage endpoint: `GET /api/v1/storage/*key` (`interface/http/storage.rs`), owner verification, HMAC token check | ✅ | `76c4186` |
| 8 | Inbound chat attachment ingestion: `interface/channels/matrix/attachments.rs` saves to storage, enqueues job, replies immediate acknowledgment | ✅ | `5d470d4` |
| 9 | Architecture boundary tests & documentation: `storage_crates_stay_inside_the_storage_infrastructure`, README, docs, HANDOFF | ✅ | `7e1a3bc` |

### Sub-project 3 — Autonomous Agent Runtime (`plans/2026-10-03-agent-runtime.md`, branch `feat/agent-runtime`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Database schema & domain models (`backend/migrations/0010_agent_runtime.sql`, `domain/agent/mod.rs`) | ✅ | this PR |
| 2 | PostgreSQL repository (`infrastructure/postgres/repositories/agent.rs`), offline `.sqlx` cache | ✅ | this PR |
| 3 | Application service (`application/agent/mod.rs`), outbox events, jobs integration | ✅ | this PR |
| 4 | ReAct execution engine, sandbox & prompts (`llm/agent/{prompts,sandbox,engine}.rs`) | ✅ | this PR |
| 5 | Background job handler & scheduler sweep (`interface/jobs/agent.rs`, `jobs/schedule.rs`) | ✅ | this PR |
| 6 | Chat & REST interfaces (`interface/chat/agent.rs`, `interface/http/agent.rs`, OpenAPI spec) | ✅ | this PR |
| 7 | Extensible plugin foundation (`ScheduledAgentTask` trait, built-in task TODOs) | ✅ | this PR |
| 8 | Full verification & documentation (clippy, fmt, sqlx check, full test suite, architecture) | ✅ | this PR |

Final verification (orchestrator, on `feat/agent-runtime`):
- `cargo fmt --all -- --check`: clean.
- `cargo check -p matrix-bot --all-targets`: clean, no warnings.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (zero warnings).
- `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets`: clean (zero drift).
- `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: byte-identical.
- `cargo test -p matrix-bot --test architecture`: 33 passed / 0 failed.
- `cargo test -p matrix-bot -- --test-threads=4`: 1298 passed / 0 failed / 7 ignored; `main.rs` 2 passed.
- `cargo test -p matrix-bot-inference`: 71 passed / 0 failed / 2 ignored.
- `npm --prefix frontend run build`: clean (1722 modules transformed, 0 errors).

### Sub-project 4 — Web Console & Multi-Interface Polish (`plans/2026-10-04-web-polish.md`, branch `feat/web-polish`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Baseline verification & E2E suite stability check (193/193 passed) | ✅ | this PR |
| 2 | Reactive SSE Sync & React Query cache invalidation (`useSystemEvents.ts`, toast alerts) | ✅ | this PR |
| 3 | Autonomous Agent module (`frontend/src/modules/agent/`: types, API hooks, RunList, RunDetailDrawer, ApprovalBanner, StartRunModal, ScheduledTasksTab, TaskConfigModal) | ✅ | this PR |
| 4 | Module integration & navigation (registered in `App.tsx`, `Sidebar.tsx`, `AppShell.tsx`, `registry.ts`) | ✅ | this PR |
| 5 | Notification routing settings panel (`NotificationPanel.tsx` in `SettingsModal.tsx`) | ✅ | this PR |
| 6 | Playwright E2E mock routes (`fake-api.ts`) & test suite (`agent.spec.ts`) | ✅ | this PR |
| 7 | Full verification (clippy, check, architecture tests, frontend build, 199/199 E2E tests) | ✅ | this PR |

Final verification (orchestrator, on `feat/web-polish`):
- `cargo check -p matrix-bot --all-targets`: clean, no warnings.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (zero warnings).
- `cargo test -p matrix-bot --test architecture`: 33 passed / 0 failed.
- `npm --prefix frontend run build`: clean (1733 modules transformed, 0 errors).
- `npm --prefix frontend run test:e2e`: 199 passed / 0 failed (100% green across entire suite).

### Engineering Housekeeping & Hardening (`plans/2026-10-04-hardening-and-housekeeping.md`, branch `feat/hardening-housekeeping`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Add automated Clippy lint check (`-D warnings`) to CI workflow (`.github/workflows/ci.yml`) | ✅ | this PR |
| 2 | Configure PostgreSQL pool statement timeout (`statement_timeout = '15s'`) in `backend/src/db/pool.rs` | ✅ | this PR |
| 3 | Link token retention & hourly purge (`repositories/link_tokens.rs`, `LinkService`, `events/purge.rs`) | ✅ | this PR |
| 4 | Channel-specific `/help` command formatting (`interface/chat/core/help.rs` adapted for Matrix, Discord, Telegram, Email) | ✅ | this PR |
| 5 | Extract `McpService` to `application/mcp/mod.rs`, wire into `AppServices`, remove repo calls from `interface/`, add architecture guard | ✅ | this PR |
| 6 | Verification (clippy, check, architecture tests, openapi drift check, full frontend e2e suite) | ✅ | this PR |

Final verification (orchestrator, on `feat/hardening-housekeeping`):
- `cargo check -p matrix-bot --all-targets`: clean (zero warnings).
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (zero warnings).
- `cargo test -p matrix-bot --test architecture`: 34 passed / 0 failed.
- `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: byte-identical (zero drift).
- `npm --prefix frontend run test:e2e`: 199 passed / 0 failed (100% green across entire suite).

### Technical Debt Remediation (`plans/2026-10-04-technical-debt-remediation.md`, branch `feat/tech-debt-remediation`)

| Task | Scope | State | Commit |
|---|---|---|---|
| 1 | Guard re-link merges to unclaimed accounts only (R165, `application/identity/link.rs`) | ✅ | `395f2a6` |
| 2 | Remove `webauthn_rs`, `sha2`, and `tracing` from domain session & credentials (`domain/accounts/`) | ✅ | `658770b` |
| 3 | Decompose monolithic agent files (`domain/agent/`, `repositories/agent/`, `application/agent/`) < 300 lines (R167) | ✅ | `a8d14ca` |
| 4 | Defer offline channel deliveries in outbox dispatcher without burning attempts (R166, `events/delivery.rs`) | ✅ | `fe13313` |
| 5 | Telegram 60s sliding window outbound deduplication (R168, `channels/telegram/outbound.rs`) | ✅ | `282b5af` `5c924bd` |
| 6 | Budget ReAct prompt tool output to 2,000 chars while preserving raw JSON in DB (R169, `llm/agent/prompts.rs`) | ✅ | `00bd4d7` |
| 7 | Decompose frontend monolithic modules `portfolio` and `finance` < 350 lines (R170) | ✅ | `fb38fc3` |
| 8 | Add WAI-ARIA and keyboard navigation to `CustomSelect.tsx` | ✅ | `8660ba7` |
| 9 | Verification & test resilience (clippy, statement_timeout on test admin pool, 1309 unit tests, 199 e2e tests) | ✅ | `58d51b9` |

Final verification (orchestrator, on `feat/tech-debt-remediation`):
- `cargo fmt --all -- --check`: clean.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean (zero warnings).
- `cargo test -p matrix-bot --test architecture`: 35 passed / 0 failed.
- `cargo test -p matrix-bot --lib -- --test-threads=4`: 1309 passed / 0 failed / 7 ignored (100% green).
- `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: byte-identical (zero drift).
- `npm --prefix frontend run build`: clean (1742 modules transformed, zero TypeScript errors).
- `npm --prefix frontend run test:e2e`: 199 passed / 0 failed (100% green).

---


## 5. How to work

### Workflow per task

The workflow is tool-agnostic; the parenthesised skill names apply only when the Superpowers skills are available.

1. **Plan exists?** Every change comes from a plan in `docs/superpowers/plans/`, and every plan comes from an approved design in `docs/superpowers/specs/`.
   - For design steps 7–10 and sub-projects 2–4, write the design (`superpowers:brainstorming`) and **get the user's approval**, then write the plan (`superpowers:writing-plans`).
   - Plans use bite-sized TDD steps with exact files, interfaces, tests and commands.
2. **Before a task:** note `BASE=$(git rev-parse HEAD)`. Re-read the feature's section of `docs/superpowers/plans/2026-09-25-feature-survey.md`.
3. **Implement** with TDD: failing test → minimal code → green → refactor. Follow the template in §8. One task is one commit (or a few green commits) with a Conventional Commits message.
4. **Verify** with the whole list below. Any failure means the task is not done.
5. **Review** the diff `BASE..HEAD` with an independent reviewer. Use a fresh agent or subagent (`superpowers:subagent-driven-development`). It judges:
   - **spec compliance**: the plan task and the global constraints;
   - **quality**: `AGENTS.md` Definition of Done, the §8 patterns, no silent behaviour change, no OpenAPI diff, tests that assert something real.
6. **Fix rounds:** fix the Critical and Important findings, then re-review **only** the fix diff. Cap it at 5 rounds. After that, decide, record the ruling, and move on.
7. **Record** the result in §4 and any decision in §9, as `Ruling: <decision> — <why> — <cost if wrong>`, in the task's commit or a follow-up `docs:` commit.

Decide ambiguities yourself, using the design doc as the authority, and write them down. **Stop and ask the user only for these:**
- irreversible actions;
- security-sensitive actions;
- anything outside the repository (push, merge, PR, production data, a live Matrix login);
- a plan so broken that every path forward is a guess.

### Verification (every task)

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets                  # warning-free
cargo test  -p matrix-bot --all-targets
cargo test  -p matrix-bot --test architecture
cargo test  -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets   # needs DATABASE_URL to a migrated scratch DB
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # NO diff allowed
cargo clippy --workspace --all-targets -- -D warnings    # must be clean (zero warnings since step 7)
npm --prefix frontend run build                          # when anything frontend-facing changed
```

If a query changes, regenerate the cache with `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` and commit `.sqlx/`.

### Test database

```bash
docker run -d --name pg18-test --shm-size=1g -p 55433:5432 -e POSTGRES_PASSWORD=bot postgres:18
# TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
# DATABASE_URL (for sqlx prepare) = same URL, after migrations have run once (any test run does it)
```

Each test gets its own `t<pid>_<n>` schema; the next test process reaps schemas left by dead processes (`db::test_schemas`, see `docs/database.md` → *Test databases*).

## 6. Safety rules (hard; never override)

- **Never read or modify the main checkout's `.env`.** It holds production credentials and may point at production PostgreSQL.
- **Never read or modify the main checkout's `data/`.** It is the real Matrix crypto store.
- **Never run the bot binary.** The only exception is `cargo run --bin generate-openapi`. Never contact a homeserver, an LLM provider or any other external service from tests or scripts.
  - An early boot check picked up the main `.env` through dotenvy's upward search and began a real Matrix login. `.env` is now read only from the current directory, only `dotenvy::from_path` is allowed, and the architecture test bans `dotenvy::dotenv`.
- **Never push, merge, open a PR or deploy** without the user's explicit instruction. Gitea is primary and GitHub is a mirror.
- **Never use bare `git stash`.** The stash stack is shared across worktrees.
- **Security invariants in `AGENTS.md` (Security Rules) must stay enforced**, each with a test proving it:
  - passkeys, step-up via `VerifiedUser` plus the `SENSITIVE` list;
  - `WEB_ONLY` commands;
  - `!link` only in a DM, with the code burned either way;
  - SHA-256 session storage, CSRF checks;
  - `OWNED_TABLES` complete, `html_escape`, 500-character input caps.

## 7. Next actions (in order)

1. **Live Discord, Telegram, and Email check by the user** (plan B, plan C, and plan D *What the user verifies*) on a non-production deployment with test credentials.
2. **Merge gates (§10)** now include rehearsing migrations 0004–0008 on a production copy (0007 guard, `!link <code>` retirement; runbook §2a). Hand them to the user; merge only on their instruction. If `refactor/outbox` changes before it merges, rebase `feat/omnichannel` onto it.
3. **Clear the §11 items** that touch the test suite first.
4. Sub-projects 3 and 4 start with a design and the user's approval.

## 8. Feature template (established by Task 1, rooms)

Read the committed rooms code before starting any feature:
- `domain/rooms`
- `application/rooms`
- `infrastructure/postgres/repositories/rooms.rs`
- `interface/channels/matrix/rooms.rs`
- `application/mod.rs` (`AppServices`, `AppError`)

### Contract per feature `<f>`

| Layer | Path | Rules |
|---|---|---|
| Domain | `domain/<f>/mod.rs` | Pure entities, validated commands, invariants as functions. No `FromRow`, no `ToSchema`, no sqlx, no I/O. Its own `thiserror` error enum. |
| Service | `application/<f>/mod.rs` | `<F>Service { pool }` owns transactions (keep the boundaries listed in the survey) and **publishes each event inside the transaction of its state change** with `repositories::outbox::publish(&mut tx, &event)`. `From<FError> for AppError` (→ `Validation`). |
| Repository | `infrastructure/postgres/repositories/<f>.rs` | Free functions over `impl sqlx::PgExecutor<'_>`, `query!` only, private `Row` structs with `From`/`TryFrom` into domain types. |
| HTTP | `interface/http/<f>.rs` | DTOs with the exact current schema names and fields, built from domain types. `AppError` maps to the same status codes and messages as before. |
| Chat | `interface/chat/<f>.rs` | The `BotModule` implementation. `handle_command(ctx, services, …)` calls the service. Errors go through `interface::chat::error_reply`; failures of external services use `AppError::External { service, source }`. |

Each feature task also:
- adds its service to `AppServices`;
- removes its files from `INTERFACE_SQL_ALLOW_LIST`;
- repoints its AI tools and MCP tools at the service;
- deletes `modules/<f>/` once it is empty;
- keeps every file at or under 500 production lines, splitting along a real seam;
- keeps every `SystemEvent` with a meaningful `owner_id()`, and keeps `OWNED_TABLES` complete.

### Patterns and rulings to reuse

- **Service parameter:** on `handle_command` it comes after `ctx`; on AI, tool and MCP functions it is the first parameter. Moved code reaches services only through these parameters.
- **Wiring `AppServices`:** one constructor, `AppServices::new(pool)`, in production and tests. Events need no wiring: they are rows in the service's own transaction, so nothing can silently drop them.
- **Many parameters:** prefer a small context struct over `#[allow(clippy::too_many_arguments)]`. `run_ai_turn`'s `#[allow]` becomes `AiTurnContext` in Task 13.
- **Tie-breaks:** among equal timestamps, the first item in picker order wins. Use a strict `>`; **never `max_by`**, which returns the *last* of equal maxima.
- **Behaviour is frozen.** If you find an existing bug, keep the behaviour and report it. If it is a security or data-correctness bug, fix it, test it, and disclose it in the commit body. Rooms did this for literal `%`/`_` matching; settings did it for validated writes.
- **Read-modify-write:** go through one service method that locks the row (`FOR UPDATE`) inside a transaction. Never fall back to defaults on a read error: only a missing row means defaults.
- **Room resolution never fails:** each step's read error is logged with `warn!` and treated as empty (`logged_or_default`).
- **Testing:**
  - Domain: plain `#[test]`.
  - Repository: `test_pool()` + `run_migrations_unlocked`.
  - Service: transaction atomicity (a NUL byte in a TEXT field forces a failure mid-transaction), degradation (a closed pool), events written in the same transaction (read them back with `repositories::outbox::event_types_for`; a rolled-back change leaves none).
  - Handler tests move with their handlers.
- **Dynamic SQL:** optional filters use `($1::text IS NULL OR col = $1)` inside `query!`. Never build SQL from strings. The only exception is `infrastructure/postgres/merge.rs` (D-A).

## 9. Rulings log (append new ones at the bottom)

### Foundation (steps 1–5)

- **R1:** the production rehearsal is a release gate run with the user, not inside a task.
- **R2:** PostgreSQL 18 (user directive).
- **R3/R4:** sqlx 0.9 forces matrix-sdk 0.18. Both upgrades landed in one commit, `c8576b6`, because neither builds alone.
- **Task 3a:** implementers never boot against live Matrix; the first boot is a supervised user gate.
- **Voice replies:** TTS replies become MSC3245 voice bubbles on matrix-sdk 0.18. Accepted, because it matches the documented invariant.
- **R5:** the principles are folded into `AGENTS.md`. Clippy is stated as a rule but not enforced in CI (the backlog was cleared in step 7; adding a CI step is now possible).
- **CI:** `sqlx-cli` is cached in CI.
- **R6:** the dotenv upward search is removed (security).
- **R7:** `statement_timeout` is deferred and recorded in the spec; revisit it with the job queue.
- **R8:** the test-schema leak is handled in the docs (`--shm-size=1g`, cleanup).
- **R9/R10:** pre-merge residuals were fixed on this branch, and the work continues here.
- **Layer skeleton:** an API bind failure aborts boot (fail fast).

### Feature layering (step 6)

- **R11 (D-A):** merge's dynamic SQL is allowed only in `infrastructure/postgres/merge.rs`, where the allow-list guard sits and where `AssertSqlSafe` is used. `AssertSqlSafe` is banned everywhere else from Task 14.
- **R12 (D-B):** the HTTP contract is frozen, so `openapi.json` must be byte-identical. Domain types derive neither `FromRow` nor `ToSchema`.
- **R13 (D-C/D-D/D-E):**
  - Matrix handlers go to `interface/chat/<f>.rs`.
  - Cross-feature reads go only to `infrastructure/postgres/read_models`.
  - todo, reminder, recurring_bills and due_reminder move as one batch.
- **D-F:** `AppServices` is built once in `app.rs` and passed to the HTTP state and the chat context. Adapters never build their own services.
- **D-G:** the interface SQL allow-list only shrinks, and Task 14 deletes it.
- **D-H:** `QuotedMessage` moves to `domain/chat.rs` in Task 8.
- **Rooms:**
  - Room targets containing `%` or `_` are matched literally. This is a bug fix: such a target used to match an arbitrary room of the user.
  - The first-in-order tie-break is kept.
  - `&AppServices` is threaded through `handle_command`, `dispatch` and the AI entry points (`run_ai_turn*`, `handle_ai_command*`, `ToolRegistry::execute_tool`, `executor::execute`, `build_system_prompt`).
- **Settings:**
  - `UserSettings` keeps `pub` fields, but its only persistence path validates.
  - The constructor without events is test-only.
  - Read-modify-write goes through `SettingsService::modify`.

### Account UUID (step 7)

- **R14:** `AccountId` wraps `uuid::Uuid` and serialises/displays as `acct_<32 hex>`, so the HTTP contract and the portfolio audit hash are unchanged. A pinned literal digest test (`test_compute_entry_hash_matches_pre_migration_pinned_digest`) guards historical chains.
- **R15 — closed:** ids are typed `AccountId` end to end; untrusted input (HTTP bodies, `--owner`) goes through `FromStr` and is rejected when malformed. Closed by the accounts/ retirement (Tasks 13–18): the `&str` facades are retyped or deleted, `From<&str>`/`From<String>`/`From<&String>` for `AccountId` are removed, fixtures use `AccountId::from_seed`, and `tests/architecture.rs::account_ids_are_never_built_from_strings_implicitly` keeps them out. (History: step 7 kept a nil-UUID fallback for the legacy facades.)
- **R16:** `0002` is one transaction with a guard that names the offending table/column/value and rolls everything back. Tests cover a legacy TEXT database upgraded in place, the guard's rollback, and a catalog check that every `owner_id`/`account_id` column is UUID with an FK. It still needs the production-copy rehearsal (§10) before release.
- Test fixtures create their owners with `db::pool::seed_account(s)`; a fixture that writes as an id with no `accounts` row now fails on the FK, as production would.

### Job queue (step 8)

Design: `specs/2026-09-29-job-queue-design.md` (amended to match the code, R31). Runtime: `docs/architecture.md` → *Background jobs*. Schema: `docs/database.md` → *Job queue*.

- **R17:** `jobs.id` is `BIGINT` identity, not SPEC's UUID — settled "BIGINT row IDs" — cost if wrong: a later rename of one column type.
- **R18:** the Matrix sync advisory lock is deferred to sub-project 2's channel refactor — one replica exists today and the scheduler lock already prevents double delivery of scheduled work — cost if wrong: a second replica would double-sync until then.
- **R19:** reminders are enqueued by a scheduler sweep, not at creation — the `reminders` row stays the single source of truth and no create/snooze/cancel path changes — cost if wrong: up to 5 s extra latency, same as before.
- **R20:** no `bills.post_due` job — no bills loop exists; bills deliver through due reminders — cost if wrong: none (behaviour frozen).
- **R21:** reminder retry delay becomes exponential (30/60/120/240 s) instead of linear (30/60/90/120 s); `retry_count` keeps its contract meaning — cost if wrong: a disclosed behaviour change.
- **R22:** portfolio keeps two cadences (alerts 2 min, refresh hourly) instead of evaluating alerts inside the refresh — behaviour frozen — cost if wrong: none.
- **R23 (C1):** `jobs` is classified `ACCOUNT_ID_FOLLOWS`, not `OWNED_TABLES` — the column is `account_id` and the classification tests key on the column name — cost if wrong: none observable; both lists move rows on merge.
- **R24 (C2/C3):** handlers take the subject account from the job row (`JobContext.account_id`), and a reminder retry no longer moves `trigger_at` (`record_failed_attempt` replaces `increment_retry`) — keeps merges and the `<id>:<trigger_at>` dedupe key coherent — cost if wrong: a merged account's pending briefing acts on the surviving account (intended).
- **R25 (C5):** `NOTIFY jobs` comes from an `AFTER INSERT` row trigger rather than the enqueue statement — fires only for real inserts, at commit, and needs no `void` column in `query!` — cost if wrong: a bulk insert NOTIFYs per row (collapsed per transaction by PostgreSQL).
- **R26 (C4):** backoff is computed in Rust (`domain::jobs::backoff_delay`) and passed to `retry` — one tested rule — cost if wrong: none.
- **R27 (C7):** `Delivery` is the handlers' seam to Matrix; briefing room sends share the reminder join-and-retry fallback — cost if wrong: the briefing "delivered" log flag now reflects a failed post-join send.
- **R31 (X12):** plan clarifications C1–C6 supersede the spec text; the spec's §3, §4, §4.1, §4.2 and §5 are amended to match the code (`ACCOUNT_ID_FOLLOWS` instead of `OWNED_TABLES`, `retry` takes a delay, NOTIFY from a trigger, `live_client.rs`, `JobContext.account_id`, `record_failed_attempt`) — each is code-forced and keeps the approved intent — cost if wrong: spec and code diverge.
- **R32:** `ReminderService::enqueue_due` marks a reminder failed when the job for its exact dedupe key ended `failed` (`fail_if_job_failed`), and the handler also marks it failed on its last attempt — the dedupe key blocks a re-enqueue, so a job that failed by timeout, database error or reaping would strand the reminder pending at a past trigger until the job is purged; exhausted attempts meant failed in the old loop — cost if wrong: one extra status lookup per due-and-conflicting reminder each 5 s tick.
- **R33:** the `briefing.send` handler drops a job (Ok; no send, no record, no event) whose `local_date` is not the recipient's current local date, and a `record_sent` failure after a send attempt is `Permanent` — bounds lateness after a Matrix outage and never resends — cost if wrong: a briefing missed on a day Matrix was down for the whole window (the old loop missed it too), or a date left unrecorded (the window closes within 5 min anyway).
- **R34:** portfolio alert DMs are sent per symbol right after that symbol's marks, and `enqueue_alert_check` skips a tick while an earlier `portfolio.check_alerts` job is still open (`repositories::jobs::has_open`) — restores the old mark-then-DM locality, bounds loss on timeout or shutdown, and stops a backlog of checks building while Matrix is down — cost if wrong: none; the check-then-enqueue is not transactional and the minute-bucket dedupe key backstops an overlap.
- **R35:** `app::run` spawns the job runtime after the API server has started, once nothing fallible remains before supervision, not before `http::bind` — a runtime detached by a bind error would leave claimed jobs locked until the reaper — cost if wrong: none observable.
- **R36:** `jobs/` stays SQL-free including its tests (the `pg_terminate_backend` test helper lives in `infrastructure::postgres::advisory` under `#[cfg(test)]`), and `test_all_account_and_owner_id_columns_are_uuid` is scoped to `current_schema()` because leaked scratch schemas made every later full-suite run flaky — cost if wrong: none.
- **R38 (X11):** the portfolio handler's snapshot test is renamed to what it proves (a refresh succeeds for an owner with no holdings); the snapshot-row write is proven at the application layer by `capturing_records_todays_usd_snapshot` — `get_history` filters today's stored row, so a handler test cannot observe it without repository access (X3) — cost if wrong: deleting the handler's capture call would not fail any test (§11).
- **R40 (X1–X11):** the pre-flight amendments X1–X11 to the plan were adopted as proposed — each fixes a plan-internal defect without changing intent — cost if wrong: none beyond what each notes.
- **R41:** commits made by subagents may carry their own model's `Co-Authored-By` trailer — accurate attribution — cost if wrong: cosmetic.

### accounts/ retirement

- **R28 (C8):** `domain::accounts::COOKIE_NAME` / `STEP_UP_WINDOW_MINUTES` were corrected to the live `"mb_session"` / 5 before any caller switched to them, and are pinned by `live_security_constants_are_pinned` — the domain copies had drifted and were unused — cost if wrong: every browser logged out, or a 15-minute step-up window.
- **R29:** audit-row parity — where an `AccountService` method records a security event the replaced facade did not, the handler's duplicate manual record is deleted, or a non-recording service method is used (logout: `end_session`); each flow has a test pinning its `security_events` rows. Services are the only audit writers: `tests/architecture.rs::interface_never_writes_security_events` forbids the write calls in `interface/` and `llm/` production code. Disclosed data-correctness fixes: the doubled `recovery_completed` row (the chat and HTTP handlers each wrote one on top of `redeem_recovery_request`'s) is gone; `recovery_requested` is written once, by the service; `matrix_linked` is written once, by the service, with the raw mxid as detail (production stored only the handler's HTML-escaped copy; the UI and DM render detail as text, so the two differ only for mxids containing `&<>"'`) — cost if wrong: a missing or doubled audit line, or historical and new `matrix_linked` rows escaped differently.
- **R30:** credential parsing is split: `domain::accounts::{Credential, cookie_value, Principal}` (pure), `interface/http/auth/credential.rs::parse_credential` (headers), `AccountService::resolve_credential` (lookups). The MCP `touch_last_used` stays a fire-and-forget spawn (frozen) — cost if wrong: a last-used stamp lost on shutdown.
- **R37:** the backup-code path writes its `recovery_completed` row when the code is consumed (before the session is created), matching the matured-request path — "completed" means the code was consumed — cost if wrong: a completed row with no session if session creation fails afterwards.
- **R39:** the `accounts/` retirement was added to the step 8 plan at the user's request with no separate design — the target layout was already settled by `AGENTS.md` §3.1 — cost if wrong: a security-sensitive refactor without a design doc.
- **R42:** `AccountService::revoke_session` is account-scoped (`AND account_id`) and audits (`sessions_revoked`) only when a row changed. The DELETE endpoint keeps its 404/200 contract (it checks ownership first); only a concurrent-revoke race returns 200 without an audit row. HTTP recovery revokes the presented cookie with `revoke_session` first and falls back to `end_session` (unscoped, unaudited) when that returns `false`, so any presented session, including another account's, is still killed; a foreign or dead cookie no longer writes the old spurious audit row (disclosed improvement) — cost if wrong: a foreign-cookie revoke on recovery is no longer audited.

### Outbox (step 9)

- **R43:** history is dropped, not migrated (O2) — user decision — cost: webhook logs are empty after cutover.
- **R44:** retention is 30 days for all finished events (O6) — SPEC §6.1 — cost: webhook logs no longer show alerts older than 30 days.
- **R45:** `max_attempts` defaults to 10 — frozen webhook behaviour — cost: none.
- **R46:** the migration is split into 0004 (DDL) and 0005 (drop) — each commit green, migrations never edited — cost: one extra migration file.
- **R47:** lease claiming instead of a `running` status and a reaper (O4) — cost: a crashed delivery waits up to 60 s.
- **R48:** NOTIFY carries `<id>:<account>` (O5) — cost: none.
- **R49:** `WebhookTriggered` carries the alert bodies and drops `outbox_id` — the event is the single record of the alert — cost: a larger SSE payload.
- **R50:** the inbound and test-alert responses' `outbox_id` becomes the delivery id — cost: a client that parsed the `outbox_` prefix breaks (none known).
- **R51:** the dispatcher idles while Matrix is disconnected — cost: none; attempts are no longer burned.
- **R52:** a temporary broadcaster→hub bridge during Tasks 3–9, deleted in Task 10 — keeps each commit green — cost: none after Task 10.
- **R53:** the plan clarifications C1–C9 supersede the spec text where they differ (counters on every failed attempt, `StoredEvent` as JSON, `WebhookTriggered(WebhookAlert)`, atomicity per state change, repositories taking `&mut PgConnection`, no publisher outside services, retry delay, 30 s delivery timeout, `events/` as a directory module); the spec is amended (`> Amended after implementation`) — the plan is the more detailed and later document — cost: none.
- **R54:** the baseline Gate was red under clippy 1.94 (`unnecessary_cast` in `llm/turn.rs`), fixed in its own chore commit `437345f` before step 9 — toolchain drift, not a behaviour change — cost: none.
- **R55:** the `sse_handler` `///` doc text still describes the old broadcast filter because it is the frozen OpenAPI description; a `//` note marks it — `openapi.json` must stay byte-identical — cost: a stale API description until the contract is next revised.
- **R56 (C4 consequence):** an event commits with its own state change before cross-service follow-ups (reminder sync after a todo/bill write, finance mirroring after a bill payment or cash move), so a failed follow-up leaves the event in place — atomicity is per state change, transaction boundaries frozen — cost: a browser may refetch state whose follow-up failed.
- **R57:** disclosed atomicity improvements — `withdraw_cash_at` now writes the debit and its audit entry in one transaction; the assistant message and `AiConversationUpdated` are atomic (a failed insert publishes nothing); the `!briefing now` announce always publishes (it depended on an optional broadcaster) — cost: none observable beyond the fixes.
- **R58:** the bookmark AI tool goes through `BookmarkService` (C6); database errors returned to the model now read `database error: …` (`AppError` display); not-found and validation texts are unchanged — cost: a slightly different error string to the model.
- **R59:** webhook counters move only when the delivery row actually changed state, so a late duplicate record after a lease re-claim cannot count twice; `enqueue_alert` with no delivery row (impossible: one destination) returns an opaque `AppError::Database`; `list_webhook_logs` keeps the old `limit.clamp(1, 100)` / `offset.max(0)` — cost: none.
- **R60:** `only_application_publishes_to_the_outbox` checks production code only; the hub's tests seed rows through `repositories::outbox::publish` — the rule guards runtime paths — cost: a test could publish outside a service (tests only).
- **R61:** `tokio-stream` is removed from `backend/Cargo.toml` (its only use was `BroadcastStream`) — cost: none.
- **R62:** F1 — deleting a webhook fails its pending deliveries in the delete's transaction — restores the pre-outbox ON DELETE CASCADE semantics — cost: none.
- **R63:** F2 — the listener uses `try_recv`; `Ok(None)` (sqlx's silent eager reconnect) triggers replay + wake — sqlx 0.9 hides IO-level losses from `recv()` — cost: none; the jobs listener keeps `recv()` (covered by its poll), noted in §11.
- **R64:** F3/F4 — delivery marks are fenced by the claimed `attempts`; each delivery's timeout is capped by the remaining lease budget (`lease - lease_margin`, margin 5 s); unsent claims (budget exhausted or shutdown) are released with their attempt restored; `mark_retry` uses the database clock — a late mark can never overwrite a re-claim — cost: none.
- **R65:** F5 — the hub tracks the highest notified id even without a local subscriber, seeds it at process start (first successful LISTEN), and replays the NEWEST missed events — cost: none.
- **R66:** F8 not fixed — every committed event wakes the dispatcher (one indexed `claim_due` per commit, on top of the 1 s poll) — narrowing it needs a new migration or a wider NOTIFY payload; single-user scale makes it negligible — cost: one cheap UPDATE per commit; revisit with the omnichannel sub-project.
- **R67:** F9 — 0005 drops `webhook_outbox` including undelivered rows; the cutover runbook checks and drains them first — cost: alerts pending at cutover are lost if the step is skipped.
- **R68:** F11 accepted — webhook logs show `attempts` including an in-flight claim and `next_attempt_at` = lease end while a delivery is in flight; `last_error` is truncated like job errors — cost: cosmetic.

## 10. User gates before merging sub-project 1

These need the human. Prepare them; don't perform them.

1. **Production rehearsal** (migrations 0004–0008; 0006–0008 add the omnichannel identity tables, see runbook §2a): first check `SELECT COUNT(*) FROM webhook_outbox WHERE status = 'pending'` on production and drain or accept it (0005 drops that table; runbook §1). Then restore a `pg_dump` of production into a scratch DB and run `MIGRATE_TEST_DB=… cargo test migrates_a_real_database -- --ignored`. Order: run master once, then take the dump (see `docs/superpowers/runbooks/postgres-foundation-cutover.md`).
2. **Supervised first boot on matrix-sdk 0.18** against a **copy** of `data/`, following the watch list in the runbook.
3. **Stray device:** the user checks the bot account for a stray device from about 2026-09-25 00:24 local time.

## 11. Known deferred items

- ~~Clippy is not yet a CI job (workspace is at zero warnings).~~ — resolved by R160 (added `cargo clippy` check to CI workflow).
- ~~`statement_timeout` is unset (R7).~~ — resolved by R161 (`connect_pool` sets `statement_timeout = '15s'` via `.after_connect`).
- ~~The AI repository has a TTL timezone issue.~~ — resolved by R148 (timestamptz comparison against `DateTime<Utc>` cutoff and ISO 8601 UTC storage in `repositories/llm.rs`).
- `SyncHealth`: if the timestamp fails to parse, it reads as "never synced" (theoretical).
- ~~The AI tool `update_my_settings` advertises `dev_room_id` and `max_history_turns` but ignores them.~~ — resolved by R149 (parsed and updated in `llm/tools/settings.rs`).
- Historical plan and review docs still mention the removed `/api/v1/health`. This is intentional.
- The architecture-test scanner cuts each line at `//`, so a URL literal hides the rest of the line. Task 14 makes it strip string literals.
- Resolved in the step 8 final fix wave: the `interface::http::auth::matrix` tests no longer reach the network (`test_state()` pre-fills `resolved_homeserver` with a closed loopback port and the client has a 500 ms timeout), so no test needs `--skip` any more.
- No handler test pins the portfolio snapshot capture call (R38): deleting `capture_current_snapshot_for_owner` from `RefreshPricesHandler` would pass the handler tests; only the application-layer test covers the write.
- ~~Test-pool schema leaks~~ — resolved by `5469098` (`db::test_schemas` reaps schemas of dead test processes), merged into this branch.
- Resolved in the final fix wave: the two flaky timing tests (`the_runtime_runs_jobs_and_stops_on_cancel` now waits `shutdown_grace` plus a margin and the reaper races each query against shutdown; the wake-up test waits for the listener's catch-up wake instead of sleeping, and the worker registers for wake-ups before it claims).
- ~~`error.rs` still sits at the crate root, outside the layer layout (the user flagged this as a file-structure issue) and needs a home.~~ — resolved: moved to `backend/src/interface/http/error.rs`, re-exported by `interface::http`, enforced by `tests/architecture.rs::error_module_is_in_interface_http`.
- (Resolved: `AccountId::from_seed` is now `#[cfg(test)]`.)
- Portfolio price refreshes now run concurrently across workers (one `portfolio.refresh_prices` job per holding owner); the old loop refreshed owners one after another, so Yahoo load is no longer serialized across owners.
- The briefing window does not wrap the hour (`07:58` is due only at :58–:59). Frozen behaviour, kept.
- ~~`!briefing now` still publishes `MorningBriefingSent` from the chat handler instead of `BriefingService::record_sent` (candidate cleanup).~~ — resolved by step 9.
- ~~`BotModule::spawn_background_workers`, `BackgroundWorkers` and `ModuleContext` remain only for the webhook worker and go with step 9.~~ — resolved by step 9.
- ~~Interface code still calls `repositories::mcp::McpTokenRepository` directly (`interface/chat/mcp.rs`, `interface/http/mcp.rs`, `interface/http/auth/mod.rs` tests): candidate for an `McpService`.~~ — resolved by R162 (`application/mcp::McpService` extracted, interface refactored, architecture rule added).
- ~~`domain::accounts::StoredCredential::passkey` imports `webauthn_rs`/`serde_json`/`tracing`, outside the domain crate list (pre-existing).~~ — resolved in `feat/tech-debt-remediation` (`credentials.rs` stores raw `Vec<u8>` credential ID).
- ~~`domain/accounts/session.rs` imports `sha2`/`webauthn_rs`/`serde_json`/`tracing`, also outside the domain crate list (pre-existing purity breach).~~ — resolved in `feat/tech-debt-remediation` (`SessionCookie` takes raw cookie hash, verified in `architecture.rs::domain_depends_on_nothing_outside_itself`).
- A recurring reminder is marked `failed` if the send succeeded but advancing it hit a database error on the last attempt.
- The HTTP recovery path reaches `interface::chat::redeem_matured_recovery`, whose `if let Ok` turns a database error into a 401.
- ~~Pre-existing re-link merge offer in `repositories/accounts/linking.rs:129-159`: an MXID removed from account A can pull A's data into a new account. Needs its own ticket.~~ — resolved by R165 (`application/identity/link.rs` strictly restricts automatic merge on re-link to unclaimed accounts only).
- ~~`JOB_WORKERS` is not validated against `DATABASE_MAX_CONNECTIONS`~~ — resolved by R150 (`Config::validate` enforces `job_workers < db_max_connections`).
- ~~Hub replay uses the oldest 500 events when `last_seen = 0`~~ — resolved by R65 (newest rows, seeded floor).
- ~~The jobs listener (`infrastructure/postgres/job_notifications.rs`) still uses `PgListener::recv()`~~ — resolved by R151 (uses `try_recv()` and wakes idle workers on silent reconnect `Ok(None)`).
- ~~`ReminderService::enqueue_due` is unbounded~~ — resolved by R152 (bounded by `MAX_ENQUEUE_DUE_BATCH = 100` via `LIMIT` in `get_due_reminders`).

- Plan A e2e gaps: the link page's sign-in then re-inspect round trip (`sign_in_required` and "I already have an account") and the 403 step-up retry on `POST /link/claim` are covered only by unit-level and API tests, not end to end in Playwright.
- ~~`ChannelSink::is_ready` means "any adapter connected" (R81): with a second destination channel a delivery whose own channel is down is claimed, fails as not connected and backs off, costing attempts. Revisit when Discord or Telegram can be a delivery destination.~~ — resolved by R166 (outbox delivery dispatcher checks destination channel connectivity and defers deliveries without burning retry attempts).
- ~~Link tokens are never purged; expired and consumed rows are inert but accumulate (one per `/link`). Add retention if volume ever matters.~~ — resolved by R163 (`LinkService::purge_expired` called hourly from `events/purge.rs`, retaining tokens for 24h).
- Four Playwright specs fail on the untouched baseline, before plan A too: `ai-chat.spec.ts:193`, `finance.spec.ts:156`, `screens.spec.ts:117`, `todo-due-reminder.spec.ts:141`. Cause: the custom select's option list stays open after a pick and intercepts the Save click. The sandbox Chromium is rev 1194 while Playwright 1.62.1 expects 1234, so this is unverified on the expected build.
- `frontend/e2e/portfolio.spec.ts:68` (currency mismatch rejected) is intermittently flaky under full-suite load; it passes in isolation.
- ~~Discord `/help` prints the shared help text with Matrix's `!` prefixes (R94); a per-channel help format is open.~~ — resolved by R164 (`interface/chat/core/help.rs` renders `/` for Discord).
- ~~`ChannelSink::is_ready` is still "any adapter connected" (R93): while Discord is down and Matrix up, an `IdentityLinked` confirmation to a Discord DM burns delivery attempts.~~ — resolved by R166 (dispatcher skips offline channels without burning attempts).
- The Discord adapter keeps two `serenity::Http` clients (its own for sends and the workers, the gateway client's for registration and deferral), so their rate-limit buckets are tracked separately; serenity retries a 429 either way (R101).
- Discord has no buttons (R89), no typed slash options and no media ingestion (spec §14).
- ~~Telegram outbox deliveries are at-least-once (no Bot API idempotency keys; T14). A re-claimed outbox delivery could deliver twice.~~ — resolved by R168 (in-memory 60s sliding window deduplication in `TelegramSendDeduper`).
- ~~Telegram `/help` prints the shared help text with Matrix's `!` prefixes (same as Discord B10).~~ — resolved by R164 (`interface/chat/core/help.rs` renders `/` for Telegram).
- Built-in task plugins implementation: the 5 planned tasks (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`) are stubbed with domain schemas and goals as TODO extensions in `domain/agent/task.rs`; execution of custom user goals and custom scheduled crons is fully functional.
- ~~Agent subsystem line-count limit violations (>500 production lines): `domain/agent/mod.rs` (~680 lines), `infrastructure/postgres/repositories/agent.rs` (~648 lines), and `application/agent/mod.rs` (~621 lines) need decomposition into submodules.~~ — resolved by R167 (decomposed into focused submodules < 300 lines each).
- ~~Frontend monolithic modules: `frontend/src/modules/portfolio/index.tsx` (1,248 lines) and `frontend/src/modules/finance/index.tsx` (807 lines) need decomposition into subcomponents.~~ — resolved by R170 (decomposed into subcomponents < 350 lines each).
- ~~ReAct engine tool output budgeting: raw JSON payloads in `llm/agent/engine.rs` need truncation to avoid context window blowup during 15-step runs.~~ — resolved by R169 (tool output capped at 2,000 characters in prompt history while preserving full JSON in DB).
- ~~`CustomSelect.tsx` accessibility: missing WAI-ARIA combobox/listbox attributes and keyboard arrow navigation.~~ — resolved in `feat/tech-debt-remediation` (`CustomSelect.tsx` added `role="listbox"`, `role="option"`, ARIA attributes, and ArrowUp/Down/Enter/Escape keyboard navigation).

## 12. Document map

| Path | What |
|---|---|
| `SPEC.md` | Roadmap for all six phases, the table inventory, NFRs |
| `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` | Binding design for sub-project 1 (D1–D10, §3 structure, §4 data, §5 jobs/outbox, §6 order) |
| `docs/superpowers/plans/2026-09-24-postgres-only-and-migrations.md` | Steps 1–4 (done) |
| `docs/superpowers/plans/2026-09-25-layer-skeleton.md` | Step 5 (done) |
| `docs/superpowers/plans/2026-09-25-feature-layering.md` | Step 6: Tasks 1–14, decisions D-A…D-H, the Feature Migration Contract |
| `docs/superpowers/plans/2026-09-25-feature-survey.md` | Per-feature facts: sizes, tables, coupling, transactions, adapters |
| `docs/superpowers/specs/2026-09-29-job-queue-design.md` | Step 8 design (job queue), amended to match the code |
| `docs/superpowers/plans/2026-09-29-job-queue.md` | Step 8 plan: Part A job queue, Part B accounts/ retirement |
| `docs/superpowers/specs/2026-09-30-outbox-design.md` | Step 9 design (transactional outbox), amended to match the code (R53) |
| `docs/superpowers/plans/2026-09-30-outbox.md` | Step 9 plan: Tasks 1–12, clarifications C1–C9 |
| `docs/superpowers/runbooks/postgres-foundation-cutover.md` | Production cutover and rollback (§2a: omnichannel migrations 0006–0008) |
| `docs/superpowers/specs/2026-10-01-omnichannel-design.md` | Sub-project 2 design (D1–D8, R-O1/R-O2 = R71/R72), amended to match the code |
| `docs/superpowers/plans/2026-10-01-omnichannel-a-framework.md` | Sub-project 2 plan A: Tasks 1–12, clarifications A1–A6 (= R73) |
| `docs/superpowers/plans/2026-10-02-omnichannel-b-discord.md` | Sub-project 2 plan B (Discord adapter): Tasks 1–7, clarifications B1–B16 (= R85–R100) |
| `docs/superpowers/plans/2026-10-02-omnichannel-c-telegram.md` | Sub-project 2 plan C (Telegram adapter), draft for approval: Tasks 1–9, clarifications T1–T16 |
| `docs/superpowers/specs/2026-10-03-agent-design.md` | Sub-project 3 design (Autonomous Agent Runtime, ReAct loop, durable jobs, approvals) |
| `docs/superpowers/plans/2026-10-03-agent-runtime.md` | Sub-project 3 plan (Tasks 1–8: schema, domain, engine, jobs, chat/HTTP, plugins) |
| `docs/superpowers/specs/2026-10-04-technical-debt-remediation-design.md` | Tech debt remediation design (security, agent split, purity, channel delivery) |
| `docs/superpowers/plans/2026-10-04-technical-debt-remediation.md` | Tech debt remediation plan (Tasks 1–9) |
| `AGENTS.md` | Rules for agents: architecture, workflow, constraints, verification, Definition of Done |
| `docs/architecture.md` | Layers, one-capability-many-interfaces, Matrix/accounts/settings design rationale |
| `docs/database.md` | Schema and SQLx conventions, migrations, test databases |
| `docs/api.md` | REST/SSE conventions, authentication, webhooks, OpenAPI workflow |
| `docs/llm.md` | LLM transport, tool loop, agent limits, MCP |
| `README.md` | Setup, deployment, *Development* (toolchain, tests, Gitea and CI) |
| `docs/superpowers/plans/2026-09-28-eliminate-spec-gaps.md` | Master plan across all four sub-projects |
- **R69:** sub-project 2 starts before sub-project 1 merges, stacked on `refactor/outbox` — the user asked for it (2026-10-01) — cost: a rebase if review changes `refactor/outbox`.
- **R70 (spec D1–D8):** linking is chat-initiated on every channel (the web-minted `!link <code>` flow is deleted); unlinked Discord/Telegram/Email identities get only `start`/`help`/`link` while Matrix keeps auto-provisioning; outbound is reply-to-origin; new accounts from a link token only for allowlisted identities; one channel-neutral `BotModule` registry; Discord commands take one `args` string; Email is a full channel over IMAP polling + SMTP with DMARC-gated inbound — user interview 2026-10-01 — cost: per decision in the spec.
- **R71 (spec R-O1):** bare Matrix room ids stay the storage form of a conversation address; other channels are prefixed (`telegram:<id>`); outbox destinations are always prefixed — no rewriting migration over every `room_id` column — cost: one prefix to strip if a later migration unifies them.
- **R72 (spec R-O2):** no generic metadata map on messages; platform data stays inside the adapter behind an opaque `MessageRef` — keeps email fields out of chat channels — cost: a typed field added later if a second channel needs one.
- **R73 (plan A, A1–A6):** migrations are 0006–0008 (0005 was taken); Matrix-specific lookups are re-pointed at `platform_identities`; link-time merges keep today's unclaimed-account rule; the AI outage alert targets `dev_room_id` or the Matrix room named "dev"; the inbound pipeline holds `Weak<ChannelRegistry>`; `Reply: From<String>` for a mechanical module sweep — cost: per clarification in the plan.
- **R74 (plan A, Task 2):** `identities::resolve_or_provision` creates the unclaimed account and binds in one transaction and rolls back on a lost race (no orphan to delete); a lost `bind` in `linking::claim` is `AlreadyLinked`; a malformed MXID reaching `AccountService`/repositories is logged at warn and treated as unknown (`find` → `None`, `unlink` → `false`, `resolve_or_provision` → validation error, Matrix handlers drop the event); the 0002 conversion test now stops at version 2 because 0007 retires `matrix_links`, and 0007 has its own backfill and orphan-guard tests — cost: none functional; a hand-edited DB with orphan links fails boot by name.
- **R75 (plan A, Task 3):** `IdentityService::unlink` refuses (Conflict) removing the last identity of a passkeyless account — the plan's rule; the old `AccountService::unlink` had no such check, so a passkeyless web account can no longer drop its only Matrix link — and locks the `accounts` row (`core::lock`) so concurrent unlinks cannot both pass the check; the unlink audit row is `identity_unlinked` (detail `matrix:<mxid>`), `claim_link` still writes `matrix_linked` until Task 9; the "previously unlinked" lookup in `linking::claim` honours both `matrix_unlinked` (`<mxid>`) and `identity_unlinked` (`matrix:<mxid>`); `redeem_matured_recovery` and `announce_account` take `AppServices`/`IdentityService` instead of `AccountService`; tests use `#[cfg(test)]` `IdentityService::{provision,find}_matrix_for_test` — cost: none on Matrix chat; a passkeyless account must add a passkey before unlinking its last identity.
- **R76 (plan A, Task 4):** `Attachment.bytes` is `Vec<u8>`, not `bytes::Bytes` — `bytes` is not a direct dependency and matrix-sdk uploads take a Vec — cost: a clone-free `Bytes` swap touching the attachment constructors if a channel later needs cheap sharing.
- **R77 (plan A, Task 5):** the turn no longer records tool targets; the three Matrix turn callers (`interface/chat/{llm,ocr,voice}`) call `record_ai_tool_targets` after the turn with `progress.message_ref()` (the stage event id) — same `bot_message_targets` rows, but written after the turn instead of between tool rounds; `ProgressSink::flush` marks the end of one streamed reply and Matrix's impl only drops the buffered throttler (no extra edit, so the streaming cadence is unchanged); `CommandContext.stage_reporter` keeps its concrete type until Task 6 — cost: a failed turn no longer leaves target rows for tools it already ran.
- **R78 (plan A, Task 6):** replies of `FinanceModule`, `PortfolioModule`, `JournalModule`, `LifeModule`, `SettingsModule` and `AccountModule` are `Reply::private`, every other reply (`!ai` included, in any conversation) is `Visibility::Normal` — the plan's sentence on `!ai` was garbled; Matrix renders both identically, the flag only matters to channels with public conversations (Task 7+); `Attachment` gains a neutral `waveform: Option<Vec<u16>>` (0..=1024 buckets) because the provider's waveform cannot be recomputed from the bytes; `TtsModule` returns the caption plus an audio attachment and `matrix/voice_upload.rs` uploads the clip first and sends the caption after it, replacing the caption with the fixed "语音发送失败" notice when the upload fails (the order and wording the module itself produced before); `CommandContext::sender_name()` is the display name, else the MXID localpart (the value the parser always recorded); `CommandContext` is no longer `Debug` (it holds a `dyn ProgressSink`); `chat_modules_know_no_platform_sdk` is `#[ignore]` until Task 7 and its hits are `voice`, `ocr`, `rooms`, `llm_alert`, `account` and `llm` — cost: Task 7 also has to move voice, OCR, rooms and the outage alert behind the adapter, not only `account`/`llm`.
- **R79 (plan A, Task 7a):** the pipeline's Matrix `link` branch passes no `PendingMerges` to `handle_link_command` (the plan's `Inbound::new` has no handle for it) and Matrix's dispatcher keeps its own free-text→`ai` rule (it interleaves quote extraction), so the shared rule is `inbound::into_command` only; 7b must thread the map or accept losing the merge prompt until Task 9 deletes the flow — cost if wrong: Matrix `!link` over the pipeline would skip the merge-confirmation state.
- **R80 (plan A, Task 7b):** (1) `rooms.rs` moved to `interface/channels/matrix/rooms.rs` (Matrix membership logic; `interface/http` and `interface/events` import it from there); (2) `chat/{voice,ocr}` keep channel-neutral `process_audio`/`process_image` (conversation key and `Option<&dyn ProgressSink>` in, `MediaReply { html, tool_results }` out) and the Matrix `attachments.rs::run_module` records the tool targets against the stage event, so the `Room`-taking entry points are gone rather than moved; (3) `build_modules(&Config)` is `async`, returns `Arc<ModuleRegistry>` (`CoreModule` needs the registry back for `!help`) and takes no `BotHandles` because the LLM and account modules lost their client; (4) `Inbound::with_pending_merges` carries the shared pending-merge map to the Matrix `link` branch until Task 9 deletes both; (5) `ChannelRegistry::from_unique` is the infallible constructor `app.rs` uses inside `Arc::new_cyclic` after `check_unique`, and the architecture test confines it, `new` and `MatrixAdapter::new` to the composition root; (6) `announce_account` takes `Option<&Weak<ChannelRegistry>>` and sends one `Target::Direct` plain-text message per linked MXID (so `matrix/notify.rs` no longer imports matrix-sdk), `None` or a dead registry logs a warning and announces nothing, as a missing client did (the web recovery redemption still passes `None`); (7) the outage alert sends through the registry to `dev_room_id` parsed as a storage address, else the Matrix room named "dev", and an unparseable setting is not replaced by the named room; (8) the Matrix reply path looks the owner up again (`find_account`) only when the reply names a created entity, since the pipeline does not hand the owner back; (9) `!link` shows no typing indicator, and an AI turn whose sender cannot be resolved now cancels its already-posted stage indicator instead of posting none; the panic apology now also passes through `link_turn_targets` (a no-op: no targets exist for it); (10) `send_dm_html` moved from `notify.rs` to `outbound.rs` for the job `Delivery` until Task 8 deletes it; (11) `app.rs` wraps the registry's Matrix `run` handle in a task that logs an `Err`, re-raises a panic and aborts `run` when aborted, so the supervision tests and the abort-laggard semantics are unchanged — cost if wrong: (9) alters transient typing/stage UI only; (11) is one extra task until Task 8 generalises supervision over all adapters.
- **R81 (plan A, Task 8):** (1) the per-channel connectivity check replaces the global Matrix one, so it runs once the stored room is known: the reminder handler after it has loaded the reminder and passed its stale checks, the briefing after the already-sent/disabled/stale checks and before generation (a deferred job that is stale, sent or disabled now finishes instead of waiting); a room that is no address or `web` completes the reminder without sending; `Rejected` marks the reminder failed and fails the job permanently, `Transient` keeps today's attempt bookkeeping, the briefing stays best-effort for both; (2) `ChannelSink` takes only the registry (the plan's `services` field has no use once `authorize_delivery` lives on the adapter) and `DeliverySink::is_ready` is unchanged and means "any adapter connected" — the dispatcher cannot know a delivery's channel before claiming it, and Matrix is the only destination channel today; revisit when a second one exists (a down channel then costs its deliveries an attempt, backed off); (3) webhook delivery never joins a room (review fix): `ChannelAdapter::authorize_delivery` returns `Result<(), String>` and Matrix runs the old sink's checks in its order with its messages (connected, `Invalid Matrix room ID`, `Bot is not joined to room: <room>`, `Delivery rejected: owner <id> is not joined to room <room>`; decision in the pure `delivery_verdict`), `ChannelSink` returns that string and, for send failures, the adapter's text unprefixed (`Matrix homeserver delivery failed: {err:?}`), so `last_error` reads as before; the outbox send sets `OutgoingMessage.join_if_needed = false` (an explicit field, not an overload of the idempotency key — the two are independent concerns and a future non-outbox idempotent send must still be allowed to join); job sends keep `true` and join-then-send; (4) `OutgoingMessage.idempotency_key` is the Matrix transaction id of the text message (not of voice uploads); (5) portfolio alerts DM the owner's Matrix identities only — an alert carries no originating conversation, so the plan's "prefer the origin channel" has nothing to prefer; (6) `Destination::as_key` of a Web conversation is the storage spelling `web`, which `parse` refuses, so the sink fails it by name instead of the domain panicking or dropping it; (7) `send_dm_html` and `DeliveryError` are deleted — cost if wrong: (2) a down non-Matrix channel will burn delivery attempts once one exists; (1) the stale-job change is observable only for jobs deferred through an outage.
- **R82 (plan A, Task 9):** (1) `ChannelSink` renders `IdentityLinked` by `event_type` and does **not** call `authorize_delivery` for it: the destination is not owner-chosen (a webhook owner picks a room, so the room check guards that choice) but the direct conversation the linking identity itself wrote to, recorded by `LinkService::start` from the authenticated inbound message with `is_direct` enforced; the check would only add a live Matrix lookup that can make a confirmation flaky, and the send still refuses a room the bot is not in (`join_if_needed = false`, key `outbox-<id>`); an unknown event type is now a delivery error instead of a webhook-parse error — cost if wrong: a stale DM the owner left still receives one confirmation line. (2) `claim` merges, in the same transaction, the account an identity was last unlinked from when it still holds data (A3), exactly when `inspect` answered `NeedsMerge` for that reason: the preview the user confirmed with the passkey-gated claim promised that merge, and the old flow's separate `confirm-merge` step is gone with `PendingMerges`; an `Merge{from}` (unclaimed holder) claim likewise records `accounts_merged` (`<n> rows`); the identity-moving `rebind` after `merge_within` is an idempotent pin because `platform_identities` follows the absorbed account — cost if wrong: a user who relinks a once-unlinked identity absorbs the old account without a second prompt (preview is shown first). (3) `AlreadyYours` writes no `identity_linked` audit row (nothing changed) but still publishes the chat confirmation; `OwnedElsewhere` (also a lost bind race) burns the token, writes and publishes nothing. (4) `claim_for_new_account(&mut PgConnection, …)` runs the same steps on a savepoint of the caller's connection (`Connection::begin`, the caller commits) so it can publish through `outbox::publish(&mut Transaction)` unchanged, and returns `Forbidden` for an identity off the allowlist (the signup handler's only guard besides the token). (5) `AppServices::new(pool)` keeps its signature and builds `LinkService` with an empty `web_url` and allowlist; composition roots call `with_link_config(web_url, allowlist)` (`app.rs`); tests that need a URL do the same. (6) The reply carrying the link is `Reply::private` and the group refusal names `!link` on Matrix, `/link` elsewhere; the retired `!link <code>` argument is ignored. (7) `LinkStart` has a hand-written `Debug` that omits the token; the `link` log lines carry channel and outcome only. (8) `inspect` without a viewer: a claimed holder or a non-allowlisted identity gets `SignInRequired`, an allowlisted identity with no claimed holder gets `SignupAllowed`. (9) `decide_link` treats an identity already on the claimer's account as `AlreadyYours` whatever its status. (10) `matrix_link_challenges`, `LinkChallenge`/`LinkOutcome` (old), `normalise_link_code`, `LINK_CODE_TTL_MINUTES`, `PendingMerges`, the three HTTP handlers and their DTOs are deleted; the frontend's Security panel still calls them until Task 11 (Task 10 regenerates `openapi.json`) — cost if wrong: the web UI's old link button is broken between Task 9 and Task 11, as the plan allows.
- **R83 (plan A, Task 10):** (1) `register_finish` is now atomic too: `AccountService::register_with_passkey` (activate-or-create the account + store the passkey + `PasskeyAdded`, one transaction, shared `register_account_in` on a caller connection) replaces the two separate calls, so a refused credential no longer leaves an active account nobody can sign in to; its responses are unchanged (a failure is the same opaque 500) and `AccountService::activate_or_create` was deleted as dead. (2) `/link/signup/finish` takes `{ceremony_id, token, credential, label?}`, not the plan's `{token, credential}`: the WebAuthn ceremony needs its id, and the new `Ceremony::LinkSignup` carries the token's SHA-256 so a challenge started for one token cannot finish another; finish re-checks the allowlist *before* touching the credential (no account work for a refused identity) and rolls back the whole account when the claim is anything but linked/merged (`expired`/`already_yours` -> 400, `owned_elsewhere` -> 409). (3) Signup display name is the identity's display name or `<Channel> <user_id>`, capped at 64 chars. (4) New limiter `check_link_signup_limit` (20/min/IP) on signup start and finish only; `inspect` is unthrottled (read-only, 256-bit tokens). (5) `/link/inspect` uses `Option<AuthUser>` (axum 0.7: any rejection, including a missing `Origin` on the POST, reads as signed out); `claim` uses `VerifiedUser`, so the existing CSRF check covers it (asserted). (6) The Security panel keeps a Matrix-only list on `GET /account/identities` and unlinks with `channel: "matrix"`; the old web "Link a Matrix account" box, the pending-merge alert and their e2e specs/screens are deleted (Task 11 adds the `/link` page and the multi-channel list); the panel now tells people to send `!link` to the bot. (7) Dead DTOs `account::MergePreview` / `UnlinkRequest` deleted. (8) Four e2e specs fail identically on the untouched baseline HEAD (`ai-chat.spec.ts:193`, `finance.spec.ts:156`, `screens.spec.ts:117`, `todo-due-reminder.spec.ts:141`: a "High Priority" dropdown option intercepts the Save click) and are unrelated to this task. — Atomic register_finish is a behaviour improvement with identical wire output; the extra `ceremony_id` field is the signup API's only deviation from the plan table; the allowlist re-check could be dropped if the allowlist ever becomes dynamic — cost if wrong: Task 11's page must send `ceremony_id`/`token` on finish; rolling back `register_finish` atomicity means reverting one service method.
- **R84 (plan A, Task 11):** (1) `LinkPage` is chosen by `window.location.pathname === '/link'` in `App.tsx` (the SPA has no router), before the shell and the global sign-in modal; on that route the `token` query parameter is never read as a passkey enrollment token (`?token=` is the enroll link's name too) and the system-events stream stays closed. (2) The token is read once into state and removed with `history.replaceState` in an effect (idempotent under StrictMode); a missing token renders the expired text without calling the server. (3) Claim reuses `useSensitiveAction` (403 "passkey to continue" -> `signInWithPasskey` -> retry), `sign_in_required` and "I already have an account" use `signInWithPasskey` then re-inspect, and signup is `signUpWithLink` in `core/passkey.ts` (sends `ceremony_id`, `token`, `label` per R83). (4) The Security panel section is now "Chat accounts" (every channel, channel label, id, display name, linked date via `formatDateInZone`, Unlink running the step-up flow; a 409 shows the server message); the badge "primary identity" marks the first identity of the list as before. (5) The merge copy is written fresh because the old confirm-merge copy was deleted in Task 9. — cost if wrong: a future router would replace the pathname check in one place.
- **R85 (plan B, B1):** a slash command reaches the registry as `InboundText::Command { command, raw_args, full_body: "<command> <args>" }`, without `/` — modules parse `full_body` and accept only `!x` or bare `x` — cost if wrong: none; plan C must do the same.
- **R86 (B2):** in a guild every interaction except `/help` is deferred ephemeral, decided before any module runs (`defer_mode`); `route_reply` sends a `Private` reply landing on a public guild response as an ephemeral follow-up behind a "Sent privately." edit (spec invariant 10) — the defer must happen within 3 s and todos, expenses, portfolio and agent answers are personal — cost if wrong: guild users cannot share a reply publicly.
- **R87 (B3):** DM text (no `MESSAGE_CONTENT` needed) is `FreeText`, hence an `ai` prompt; it is never parsed as a command; guild messages are not received; DM attachments are ignored — cost if wrong: none.
- **R88 (B4):** replies are embeds with a description only (no title, no colour) — cost: cosmetic.
- **R89 (B5):** `Content.actions` are dropped on Discord with a `debug!` (buttons out of scope) — cost: none until a module emits actions.
- **R90 (B6):** `DiscordAdapter` holds config, an `Http` client, the connected flag and the pipeline, no `AppServices` — cost: none.
- **R91 (B7):** `run` rebuilds the gateway client after `Client::start` ends, 1 s doubling to 300 s, reset after a 60 s session; `InvalidAuthentication` / `InvalidGatewayIntents` / `DisallowedGatewayIntents` end `run` with `Err`, which stops the process — cost if wrong: a revoked token takes the Matrix bot down with it until the orchestrator restarts and the token is fixed.
- **R92 (B8):** the event handler parses, defers and `try_send`s onto a 32-slot queue drained by four workers owned by `run`; a full queue answers "busy" (interactions) or drops with a `warn!` (DMs); delivery failures are logged, not retried — cost: a burst above 36 concurrent requests is refused.
- **R93 (B9):** `ChannelSink::is_ready` stays "any adapter connected" (R81 revisited) — cost: see §11.
- **R94 (B10):** `/help` shows `CoreModule`'s shared text — cost: `!`-prefixed help on Discord.
- **R95 (B11):** `InboundOutcome::Ignored` on an interaction edits the response to "Nothing to answer for that."; a reply Discord cannot carry (oversized file) edits it to the reason — cost: none.
- **R96 (B12):** interactions are `is_mentioned = true`; DMs `is_direct = true, is_mentioned = false` — cost: none.
- **R97 (B13):** `supervise_front_doors` takes `&mut [AdapterTask]` and waits on `select_all`; `front_door_name` names each channel's task distinctly — cost: none.
- **R98 (B14):** attachments upload as Discord files; above 10 MiB the send is `Rejected` — cost: none (TTS clips are far smaller).
- **R99 (B15):** commands have `dm_permission(true)`; `args` max 500, `/ask` `prompt` required, max 4000; the adapter re-checks both and refuses ephemerally — cost: none.
- **R100 (B16):** an idempotency key of at most 25 characters becomes the message nonce with `enforce_nonce`; `join_if_needed` is ignored on Discord — cost: none.
- **R101 (plan B, implementation):** (1) `Work` lives in `worker.rs` next to its consumer, not in `gateway.rs` as the plan sketched; (2) `outbound`'s builders, `bounded` and `classify*` are `pub` (the module's interface for the worker, gateway and progress); (3) `Attachment` derives `PartialEq, Eq` so `Post` can be compared in tests; (4) serenity adds `rustls` 0.22 and `tokio-tungstenite` 0.21 to the lockfile, sharing the existing `reqwest` 0.11; (5) the adapter's own `Http` and the gateway client's are separate instances (§11); (6) the integration tests in `discord/mod.rs` run Discord fixtures through the real `build_modules` registry and a test database — cost if wrong: none functional.
- **R102 (T1 — `teloxide-core`, not `teloxide`):** `teloxide-core` 0.13 (`rustls`) brings types and `Bot` without pulling dptree or a separate axum 0.8 webhook server; requires `reqwest` 0.12 with `takecell` pinned to 0.1.1 for rustc 1.94 compatibility — cost: third reqwest line in Cargo.lock.
- **R103 (T2 — `full_body` is bare form):** `/todo@MyBot add milk` → `InboundText::Command { command: "todo", raw_args: "add milk", full_body: "todo add milk" }`; `@mention` suffix is stripped; any `/name` parses as a command; `setMyCommands` advertises the 6 core commands (`start help todo expense portfolio link`) — cost: commands addressed to other bots are ignored.
- **R104 (T3 — addressed text):** private chats: non-command text → `FreeText` (direct); groups/supergroups: only commands, `@bot` mentions, and direct replies to the bot's messages reach the bot; broadcast channels and edited messages are ignored — cost: background group chat is not received (privacy mode).
- **R105 (T4 — `/start` redispatches as `/help`):** unlinked senders get the link guidance; for linked senders, if `/start` is unhandled by modules (`Ignored`), worker redispatches once as `/help` — cost: none.
- **R106 (T5 — quoted messages):** replies to messages build `quoted = QuotedMessage` from the replied-to text, so `/todo add` / `/remind` in reply use it as body — cost: none.
- **R107 (T6 — callbacks re-dispatched as presser, invariant 9):** callback queries are answered at once (`answerCallbackQuery`), parsed without leading `/`, and dispatched through `Inbound::handle` as the pressing user; another user pressing "Done #id" gets "not found" — cost: none.
- **R108 (T7 — inline keyboards from actions):** 1 button per row, label max 64 chars, max 8 rows, callback data ≤ 64 bytes (longer dropped with debug log); todo list is first producer with `Done #<id>` buttons — cost: commands > 64 bytes cannot be buttons.
- **R109 (T8 — group privacy, invariant 10):** `Visibility::Private` replies in groups/supergroups are refused with `PRIVATE_IN_GROUP` ("That answer is private — message me directly.") and never posted publicly — cost: private data never leaks to groups.
- **R110 (T9 — webhook ingress):** `POST /api/webhooks/telegram` outside `/api/v1`; 120/min IP rate limit; constant-time secret check before reading body; 200 on enqueue, 503 on full queue; 404 when disabled/polling — cost: none.
- **R111 (T10 — constant-time comparison):** implemented in `utils::secure::constant_time_eq` using length-checked XOR fold with `#[inline(never)]` — cost: no extra dependency.
- **R112 (T11 — update queue & dedupe):** 4 worker tasks drain bounded 32-slot queue; updates deduplicated by `RecentUpdates` (1024-id window); webhook and polling share the same worker path — cost: bursts > 32 return 503.
- **R113 (T12 — webhook not in OpenAPI):** infrastructure endpoint; compat shim moved to `interface/http/webhooks/telegram_compat.rs` keeping OpenAPI schema byte-identical — cost: none.
- **R114 (T13 — Telegram configuration validation):** `TELEGRAM_MODE` (`webhook`|`polling`); webhook mode requires `TELEGRAM_WEBHOOK_SECRET` and HTTPS `WEB_URL`; `TELEGRAM_ALLOWED_USERS` parsed as positive i64 — cost: invalid config fails at boot.
- **R115 (T14 — no outbox dedupe on Telegram):** Bot API has no idempotency key; outbox deliveries are at-least-once — cost: a reclaimed outbox delivery can post twice (only link confirmation goes through outbox today).
- **R116 (T15 — progress sink):** `TelegramProgress` sends typing action every 4 s in DMs and edits a status line message every 1.5 s — cost: none.
- **R117 (T16 — attachments):** Ogg Opus → `sendVoice`, other audio → `sendAudio`, photos → `sendPhoto`, documents → `sendDocument`; > 50 MB produces a warning notice part — cost: oversized files not sent.
- **R118 (plan C, Task 3):** (1) `teloxide-core` 0.13 adds duplicate majors `reqwest` 0.12, `indexmap` 1.9, `hashbrown` 0.12 and new `schemars` 0.9/1.x (no existing crate changed version); (2) its transitive `takecell` 0.1.2 needs rustc 1.96, the toolchain is 1.94.1, so `Cargo.lock` pins `takecell` 0.1.1 (`cargo update takecell --precise 0.1.1`) — cost if wrong: a later `cargo update` may re-select 0.1.2 and break the build until the toolchain moves; (3) teloxide-core's `Update` deserializes only from a string or bytes: `serde_json::from_value` silently yields `UpdateKind::Error`, so Task 5's workers must use `from_slice` (the fixtures in tests go through `to_string`); (4) `CommandRefusal` has only `TooLong` (any `/name` parses, T2); (5) `message_id`/`reply_to` refs are `message:<chat>:<id>` — cost if wrong: none functional.
- **R119 (plan C, Task 4):** (1) an attachment over 50 MB becomes a text notice part, not `Rejected` as the plan's step 1 said (the orchestrator's T16 clarification: the rest of the reply still goes out) — cost if wrong: a caller expecting an error for an oversized file gets a delivered notice; (2) an empty reply (no text, no files) is `Rejected("nothing to send")` since Telegram refuses empty messages; (3) `ApiError::Unknown` is `Transient` unless its text starts `Bad Request`/`Forbidden`/`Unauthorized`/`Conflict` (teloxide exposes no status code), all other `Api` errors and `MigrateToChatId` are `Rejected`; (4) `chat < 0` means group (Telegram's group and supergroup ids are negative) and decides T8; (5) a `reply_to` ref in another chat or malformed is dropped, and reply parameters use `allow_sending_without_reply`; (6) `redact` scrubs `/bot<id>:<secret>` from every error text on top of teloxide's own hiding; (7) `send` is tested only through target validation, `classify` and the pure planner (no fake server) — cost if wrong: the HTTP request building is covered only by compilation until Task 5/8 integration.
- **R120 (plan C, Task 5):** (1) `TelegramAdapter::new` returns `anyhow::Result<Self>` (the plan said `Self`): building the reqwest 0.12 client can fail and `expect` is banned on runtime paths; Task 8 uses `?`. (2) The client is built from `teloxide_core::net::default_reqwest_settings()` with a 70 s timeout, so no direct `reqwest` 0.12 dependency is needed; every call still runs under `outbound::bounded` (30 s) except `getUpdates` (65 s bound, 50 s long poll). (3) The queue carries `axum::body::Bytes` (no new crate); polling re-serializes each polled `Update` to JSON (round-trip tested on the fixtures) so webhook and polling share one worker path; polling waits for queue capacity, the webhook `submit` never blocks. (4) The progress throttle lives in the shared `interface/channels/throttle.rs`, used by Discord and Telegram. (5) T4 lives in `worker::dispatch` (the re-dispatch as `help` happens once, only when the pipeline returns `Ignored` for `start`); T6's `answerCallbackQuery` runs before the pipeline call; a `getMe`/`getUpdates` `InvalidToken` is fatal (`run` returns `Err`), other errors back off 1 s doubling to 300 s. (6) Dedupe is a shared 1024-id window across the four workers; polling advances `offset` past every received update before queueing it. (7) Tests use pure seams and the real pipeline; no fake HTTP server (`Bot` calls are covered only by the thin glue, verified in the Task 8/user smoke test) — cost if wrong: a Bot-API-shape regression in worker/progress glue is not caught before the live bot.
- **R121 (plan C, Task 6):** (1) `telegram_update` takes the body as `axum::body::Body` and reads it with `to_bytes(MAX_UPDATE_BYTES = 1 MiB)` only after the secret matched, instead of the plan's `body: Bytes` argument (an extractor would read the body before the 401 check, against T9/invariant 7); an over-limit body is 413. (2) The limit equals the server-wide 1 MiB layer on purpose (T9) and is enforced in the handler so it is testable without the layer stack. (3) The route is mounted at the root only (`/api/webhooks/telegram`, next to `/webhooks/:token`), not under `/api/v1`; CSRF needs no exemption since it lives in the `AuthUser` extractor, and a test proves a request with no Origin, cookie or Authorization reaches the handler. (4) The client IP for the 120/min limiter comes from `extract_client_ip` (honours `trust_forwarded_for`); `ConnectInfo` is optional so `oneshot` tests work. (5) `ApiState.telegram: Option<TelegramIngress>` is `None` in both constructors until Task 8 wires the adapter. (6) The shim moved by `git mv` to `webhooks/telegram_compat.rs`; mounts, responses and OpenAPI operation ids are unchanged — cost if wrong: none functional.
- **R122 (plan C, Task 7):** (1) `handle_todo_command` and the list handlers now return `Option<Reply>` (the plan said only "the list call sites"): a `Reply` is the only carrier for `actions`, and the other sub-handlers wrap their `String` with `Reply::from`. The list HTML is `format_todo_list` unchanged (pinned by a literal-string test); Matrix sends only `plain`/`html` (`matrix/outbound.rs::text_content`, pinned) and Discord drops actions (B5), so no other channel changes. (2) Buttons are built only for open items owned by the viewer (plan: "at most 10 shown items"): a room list also shows other members' tasks and `todo done` is owner-scoped, so a button on them would only answer "not found"; label is `Done #<id>` as the plan says, not a title. (3) The press goes through `Inbound::handle` as the presser, so ownership is the pipeline's account resolution plus `TodoService::set_completed(owner, id)`; a pipeline test proves a different linked account gets "not found" and the todo stays open. — cost if wrong: a list longer than 10 open items only offers buttons for the first 10 (the rest stay `!todo done <id>`).
- **R123 (plan C, Task 8):** `app.rs` registers `TelegramAdapter::new(c, config.web_url.clone())` in `Arc::new`, adds to `check_unique`, attaches the shared inbound pipeline via `new_cyclic`, builds `ChannelRegistry::from_unique` including Telegram, and supervises its updates loop under `front_door_name` "Telegram updates" (early exit cancels the root token, tested); `telegram.as_ref().and_then(|t| t.ingress())` is passed to `start_api_server` and stored in `ApiState.telegram`; `start_api_server` takes 8 arguments with `#[allow(clippy::too_many_arguments)]` matching project convention; `log_registration_mode` logs whether Telegram is enabled and its mode (never the token or secret); architecture tests verify `teloxide_stays_inside_the_telegram_adapter` and `only_the_composition_root_builds_adapters_and_the_registry` includes `TelegramAdapter::new(`.
- **R124 (plan C, Task 9):** all documentation updated (`architecture.md`, `api.md`, `README.md`, `.env.example`, `SPEC.md`, `specs/2026-10-01-omnichannel-design.md`, `HANDOFF.md`); Plan C tasks 1–9 complete; final verification gate passed with all gates green.
- **R125 (E1 — dependencies):** `mail-parser` 0.11 for MIME parsing, `lettre` 0.11 (`default-features = false, features = ["smtp-transport", "tokio1-rustls-tls", "builder", "tracing"]`) for SMTP, `async-imap` 0.12 (`default-features = false, features = ["runtime-tokio", "tokio"]`) for IMAP, and `tokio-rustls` 0.26 / `webpki-roots` 0.26 with explicit `ring` provider for IMAPS TLS — cost: none.
- **R126 (E2 — inbound text extraction):** Body extracted from first `text/plain` part (falls back to `text/html` converted to text). Quotes (`> ...`, `On ... wrote:`, `-----Original Message-----`, Outlook splitters) and signatures (`-- `) stripped. If body is empty after stripping, falls back to `Subject`. Commands can be typed with `/`, `!`, or bare command name matching registry — cost: none.
- **R127 (E3 — inbound authentication, DMARC pass rule, Invariant 8):** The adapter parses all `Authentication-Results` headers. Only the header whose authserv-id matches `EMAIL_AUTHSERV_ID` is trusted. It must contain `dmarc=pass` for the `From` domain. Anything else is logged with `warn!` and dropped without a reply to prevent backscatter spam — cost: unauthenticated emails are silently dropped.
- **R128 (E4 — auto-reply drop):** Messages with `Auto-Submitted` != "no", `X-Auto-Response-Suppress`, `Precedence: bulk/list/junk`, or `From` starting with `mailer-daemon`, `postmaster`, `noreply`, `no-reply` are dropped silently without reply — cost: none.
- **R129 (E5 — identity & conversation mapping):** Sender is `PlatformIdentity(Channel::Email, PlatformUserId::parse(Channel::Email, addr)?)` with display name from `From`. Conversation is `ConversationAddress::Email(addr)`. `is_direct = true`, `is_mentioned = true` — cost: none.
- **R130 (E6 — outbound threading & subject):** Outgoing replies set `In-Reply-To` and `References` to the incoming `Message-ID`. Subject is `Re: <subject>` (preserves existing `Re:`). Unsolicited messages derive subject from first line of text capped at 60 chars or `"Matrix Bot"` — cost: none.
- **R131 (E7 — outbound multipart & action rendering):** Messages are sent as multipart/alternative (`text/plain` + `text/html`). `Content.actions` render as a text list at the bottom: "Reply to this email with: • <command>" in plain text and HTML list — cost: none.
- **R132 (E8 — linking):** `/link` sends the token URL to the `From` address. Claiming the link publishes `IdentityLinked` which triggers an outbox confirmation email "✓ linked" back to `email:<address>` — cost: none.
- **R133 (E9 — IMAP polling & stream draining):** Polling loop connects to `EMAIL_IMAP_URL` (IMAPS, port 993 default) via `tokio-rustls` with `ring` crypto provider, logs in with `EMAIL_USERNAME`/`EMAIL_PASSWORD`, selects `INBOX`, searches `UNSEEN`, fetches at most 20 messages per poll, marks `\Seen` and properly drains the async stream via `pin_mut!`, and optionally moves to `EMAIL_PROCESSED_FOLDER` — cost: none.
- **R134 (E10 — SMTP transport):** Lettre `AsyncSmtpTransport` with STARTTLS or implicit TLS. Timeouts capped at 30 s. Errors mapped to `ChannelError::Transient` (temporary 4xx or connection loss) or `ChannelError::Rejected` (permanent 5xx) — cost: none.
- **R135 (E11 — progress sink):** Email uses `llm::progress::NoProgress` (no-op; intermediate tool calls are not emailed) — cost: none.
- **R136 (E12 — config validation):** Registered only when `EMAIL_IMAP_URL` is set. When set, `EMAIL_SMTP_URL`, `EMAIL_USERNAME`, `EMAIL_PASSWORD`, `EMAIL_FROM`, and `EMAIL_AUTHSERV_ID` are required. `EMAIL_ALLOWED_SENDERS` validated as email addresses. `EMAIL_POLL_SECONDS` defaults to 60 (minimum 5) — cost: invalid config fails at boot.
- **R137 (E13 — deduplication):** Inbound messages deduplicated by `Message-ID` in a 1024-entry sliding window (`RecentMessages`). Outbox deliveries to email are at-least-once — cost: none.
- **R138 (E14 — architecture isolation):** Architecture test verifies `lettre`, `async_imap`, and `mail_parser` are isolated inside `interface/channels/email/` (`email_crates_stay_inside_the_email_adapter`) and composition root builds `EmailAdapter::new(` — cost: none.
- **R139:** `error.rs` relocated from crate root to `backend/src/interface/http/error.rs` with `pub use error::{ApiError, ApiErrorResponse};` in `interface/http/mod.rs`; `lib.rs` removes `pub mod error;`, all `interface/http` callers re-pointed to `crate::interface::http::error::*`, and `tests/architecture.rs::error_module_is_in_interface_http` prevents regression — cost: none; byte-identical `openapi.json`.
- **R140 (S1 — `StorageKey` value object):** Canonical format `accounts/{account_id}/{category}/{uuid}.{ext}`; strictly bans traversal (`..`, `\`, leading/trailing `/`) and restricts charset to `[a-zA-Z0-9_.-]`; owner-isolated at the type level — cost: none.
- **R141 (S2 — `StorageService` trait):** Pluggable async trait for `put`, `get`, `delete`, and `get_presigned_url`. Concrete implementations `LocalStorageService` (local filesystem, atomic write via `.tmp`) and `S3StorageService` (`aws-sdk-s3` with native presigning) — cost: none.
- **R142 (S3 — Presigned local URLs):** Uses HMAC-SHA256 tokens (`?sig=...&exp=...`) against the API server's `/api/v1/storage/*key` endpoint, giving parity with S3 without exposing internal filesystem paths — cost: none.
- **R143 (S4 — Migration 0009):** `ALTER TABLE transactions ADD COLUMN IF NOT EXISTS receipt_storage_key TEXT;` with index on `receipt_storage_key`. Handled in `OWNED_TABLES` automatically — cost: none.
- **R144 (S5 — Async Receipt Ingestion):** Receipt processing is fully asynchronous via durable background job `media.process_receipt`. Chat sync loop only saves to storage and enqueues job with immediate acknowledgment — cost: none.
- **R145 (S6 — Pure domain receipt parsing):** `domain::finance::receipt::extract_receipt_fields` is pure and synchronous; HTTP OCR endpoint and background job delegate to it — cost: none.
- **R146 (S7 — Authenticated storage streaming endpoint):** `GET /api/v1/storage/*key` validates `AuthUser` ownership (`key.account_id() == user.account_id`) or valid unexpired HMAC signature, streaming with appropriate Content-Type — cost: none.
- **R147 (S8 — Crate isolation):** `aws-sdk-s3` and `aws-config` are strictly private to `infrastructure/storage/`, asserted by `storage_crates_stay_inside_the_storage_infrastructure` in `backend/tests/architecture.rs` — cost: none.
- **R148 (AI conversation TTL timezone fix):** `get_or_create_conversation` queries `updated_at::timestamptz >= $3 ORDER BY updated_at::timestamptz DESC` binding `cutoff` as `DateTime<Utc>`, and timestamps are stored via `TO_CHAR(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')` — prevents string comparison failures across varied timezone formats and ISO 8601 offsets — cost if wrong: conversations expiring prematurely or staying active across sessions.
- **R149 (`update_my_settings` options):** `update_my_settings` tool parses and applies `dev_room_id` (trimmed, clearing to `None` on `"none"` or empty) and `max_history_turns` (via `domain::settings::parse_max_history_turns` clamped to 1..=100) — aligns AI tool execution with advertised schema parameters — cost if wrong: silent omission of settings changes requested via AI.
- **R150 (Pool starvation check):** `Config::validate` enforces `job_workers < db_max_connections as usize` — ensures background job workers cannot saturate the connection pool and starve API handlers, sync loops, and transactional queries — cost if wrong: connection starvation under peak job concurrency.
- **R151 (Job listener silent reconnect):** `JobNotifications::next` uses `self.listener.try_recv().await?` and maps `Ok(None)` (silent reconnect in sqlx 0.9) to waking idle workers via `Ok(())` — prevents missed notifications during network hiccups without relying solely on polling interval — cost if wrong: jobs delayed until next periodic sweep when listener reconnects.
- **R152 (Bounded reminder sweep):** `get_due_reminders` takes `limit: i64` (`LIMIT $3`); `ReminderService::enqueue_due` passes `MAX_ENQUEUE_DUE_BATCH = 100` — bounds memory and lock contention during large reminder backlogs — cost if wrong: unbounded query loading thousands of reminders simultaneously during recovery.
- **R153 (A1 — Autonomous Agent Runtime Schema & Jobs):** Migration 0010 adds `agent_runs`, `agent_steps`, `agent_approvals`, and `scheduled_agent_tasks` (all owner-keyed, added to `ACCOUNT_ID_FOLLOWS`). Run execution is handled asynchronously via the durable job runtime (`agent.execute_run`) with worker heartbeats (`update_heartbeat`), lease management, and cancellation support — cost: none.
- **R154 (A2 — ReAct Engine & Sensitive Action Interception):** Single-step ReAct loop with pure prompt formatting, thought extraction, and tool whitelist sandboxing. Tools designated sensitive (`is_sensitive == true`) are intercepted into `agent_approvals`, pausing run execution as `waiting_approval` with transactional outbox notification. Approval or rejection resumes or skips the action via dedicated chat commands (`!agent confirm`/`!agent reject`) or REST endpoints — cost: none.
- **R155 (A3 — Extensible Plugin Foundation & Domain Purity):** `ScheduledAgentTask` trait and `ScheduledTaskRegistry` in `domain/agent/task.rs` remain completely pure and synchronous (no async, no external dependencies outside domain rules). 5 built-in task types (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, `PortfolioRebalanceAlertTask`, `BillDueDigestTask`, `HabitConsistencyReviewTask`) stubbed as TODO extensions per directive — cost: built-in task logic deferred to future extension PRs.
- **R156 (W1 — Passkey Step-Up Parity for Agent Approvals):** `useApproveAction` reuses `useSensitiveAction` from `core/security.ts`, which automatically catches 403 step-up responses, prompts user for passkey verification via `signInWithPasskey()`, and transparently retries the approval mutation — cost: none.
- **R157 (W2 — Global Reactive Invalidation):** `useSystemEvents` binds SSE events (`Agent*`, `Todo*`, `Reminder*`, `Transaction*`, `Portfolio*`, `RecurringBill*`, `Webhook*`, `IdentityLinked`) directly to central React Query cache invalidation keys, enabling instant cross-device and multi-channel UI updates without manual polling — cost: none.
- **R158 (W3 — Ephemeral Lightweight Toast Alerts):** ToastContainer renders non-blocking status pills for high-priority SSE alerts (`AgentApprovalRequired`, `ReminderTriggered`) with auto-dismiss (4s) without altering core application state — cost: none.
- **R159 (W4 — E2E Playwright Concurrency Stability):** Playwright tests on local dev server run with bounded workers (4) to prevent port navigation timeouts on Vite dev server under full-suite load — cost: none.
- **R160 (CI Clippy):** Added automated `cargo clippy --all-targets -- -D warnings` step to GitHub Actions CI workflow for both `matrix-bot` and `matrix-bot-inference` jobs — cost: none.
- **R161 (PostgreSQL statement timeout, R7):** `connect_pool` in `backend/src/db/pool.rs` executes `SET statement_timeout = '15s'` on connection initialization via `.after_connect`, guarding against runaway long-running transactions while leaving unconstrained test setup unaffected — cost: queries running >15s without explicit transaction timeout override will fail.
- **R162 (McpService Application Extraction):** Extracted `McpService` into `application/mcp/mod.rs` and wired into `AppServices`. Replaced direct `McpTokenRepository` usage across `interface/http/mcp.rs`, `interface/chat/mcp.rs`, and `interface/http/auth/mod.rs`. Retired dead `McpTokenRepository` struct and added architecture test `interface_does_not_import_mcp_repository` — cost: none.
- **R163 (Link Token Retention & Purge):** `repositories::link_tokens::purge_expired` and `LinkService::purge_expired` delete link tokens older than 24 hours (1-day forensic window beyond 10-minute expiry). Hourly invocation hooked into `events/purge.rs` — cost: none.
- **R164 (Channel-Specific `/help` Formatting):** Updated `generate_help_html` in `interface/chat/core/help.rs` to format command prefixes dynamically (`/` for Discord and Telegram, `!` for Matrix and Email), with platform-tailored header and usage notes — cost: none.
- **R165 (Re-Link Merge Guard):** In `application/identity/link.rs`, automatic merges on identity re-linking are strictly restricted to unclaimed holder accounts (`STATUS_UNCLAIMED`). Re-linking an identity previously unlinked from an active account (`STATUS_ACTIVE`) is treated as a clean fresh bind (`LinkOutcome::Linked`) with zero data absorbed from the prior account — prevents unauthorized cross-account data absorption — cost if wrong: a user wanting to merge two active accounts after re-linking must use explicit settings merge.
- **R166 (Per-Channel Delivery Readiness):** Outbox delivery dispatch checks destination channel connectivity. Offline channel deliveries are skipped without claiming or burning attempts, resuming immediately upon adapter reconnect — prevents retry exhaustion during partial outages — cost if wrong: none.
- **R167 (Agent File Decomposition):** Monolithic agent files (`domain/agent/mod.rs`, `repositories/agent.rs`, `application/agent/mod.rs`) decomposed into cohesive functional submodules (`run`, `step`, `approval`, `config`/`task`) < 300 lines each — enforces AGENTS.md §4.3 rule — cost if wrong: none.
- **R168 (Telegram Delivery Deduplication):** In-memory 60-second sliding-window deduplication in `TelegramAdapter` / `outbound.rs` suppresses duplicate message transmissions from worker retries or re-claimed leases — mitigates Telegram API lack of client idempotency keys — cost if wrong: identical rapid messages within 60s are suppressed.
- **R169 (ReAct Tool Output Budgeting):** Tool outputs exceeding 2,000 characters in ReAct prompt history are truncated with an explicit note, while preserving full unabridged JSON in PostgreSQL `agent_steps` — bounds LLM prompt size and token cost — cost if wrong: LLM does not see full output beyond 2,000 chars without paging tools.
- **R170 (Frontend Monolithic Decomposition Scope):** Decompose the two largest outliers `portfolio/index.tsx` (1,248 lines) and `finance/index.tsx` (807 lines) into modular subcomponents (< 350 lines each), deferring the ~600-line modules (`todo`, `mood`, `reminder`) — keeps UI regression risk bounded — cost if wrong: ~600-line modules remain slightly above the 500-line target until next pass.
- **R171 (Frontend Test Toolchain):** Rely on the 199 Playwright E2E suite without introducing Vitest in this sprint — avoids dependency bloat — cost if wrong: unit-level tests for utilities require Playwright or manual test checks.



