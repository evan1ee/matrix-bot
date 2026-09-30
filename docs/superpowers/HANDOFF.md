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
| 2 — Omnichannel | 4 | `ChannelAdapter` / `ChannelRegistry`; Matrix refactored onto it; Discord (`serenity`) and Telegram (`teloxide`) adapters; `platform_identities`; one tool registry shared by every channel | Not designed |
| 3 — Scheduled agent & storage | 5 | `ScheduledAgentTask` runtime (`scheduled_agent_tasks`, `agent_runs`, `agent_steps`); `StorageService` (local / S3); HTTP contracts between the backend and inference crates standardised | Not designed |
| 4 — Web polish | 6 | Regenerated TypeScript client; live UI through the unified SSE stream; UI for linked identities and notification routing | Not designed |

The order is strict. Each sub-project builds on the previous one: channels need the layers and the tool registry, the agent needs the job queue, and the web UI needs the outbox events. **Do not start sub-project 2 until sub-project 1 is merged**, unless the user says otherwise.

The **React frontend** (`frontend/`) is essentially untouched by sub-project 1: it only switched to the `/health/*` endpoints. The public HTTP contract is frozen, so `backend/openapi.json` must not change.

## 2. Target architecture (sub-project 1)

The backend follows a layer-first layout (`SPEC.md` §13; design §3):

```
backend/src/
  main.rs            env (.env from the current directory only) → app::run
  app.rs             composition root: config, pool, migrations, AppServices, front doors,
                     one CancellationToken tree, 15 s bounded shutdown, supervision
  domain/<f>/        pure types + invariants (std/chrono/uuid/serde/thiserror only)
  application/<f>/   <F>Service: use cases, transactions, publishes events after commit
                     application/mod.rs: AppError, AppServices
  infrastructure/
    postgres/        pool, ping, repositories/<f>.rs (sqlx::query! only), read_models/, merge.rs
    inference/       HTTP clients to the inference crate (OCR, STT, TTS) and the LLM
  interface/
    http/            axum handlers + DTOs (the OpenAPI types), health probes
    chat/            BotModule command handlers per feature, error_reply
    jobs/            job handlers: reminder.fire, briefing.send, portfolio.* (payload → service → Delivery)
    channels/matrix/ matrix-sdk client, sync loop, E2EE, verification
  jobs/              job runtime: workers, reaper, purge, leader-elected scheduler (no SQL)
  llm/               prompts, tool loop, tool registry
  error.rs events.rs at the crate root for now (§11); events/ is added by design step 9
```

**Dependency rule** (enforced by `backend/tests/architecture.rs`):

| Layer | May not import |
|---|---|
| `domain` | anything except the pure crates listed in the tree above |
| `application` | axum, matrix-sdk, reqwest, utoipa, `interface` |
| `infrastructure` | `application`, `interface` |
| `jobs` | axum, matrix-sdk, utoipa, reqwest, `interface`; no SQL, tests included |

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
| 9 | Outbox (`0004_outbox`, `outbox_deliveries`, SSE via LISTEN; remove `EventBroadcaster` and `webhook_outbox`). Design `specs/2026-09-30-outbox-design.md` (approved 2026-09-30), branch `refactor/outbox`, plan `plans/2026-09-30-outbox.md` (Tasks 1–12, clarifications C1–C9) | ◐ implementation in progress: Tasks 1–4 of 12 done | — |
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

The last full test run (step 8, after merging the test-schema reaper, `cargo test -p matrix-bot --all-targets --no-fail-fast`, no `--skip`): **lib 859 passed / 0 failed / 7 ignored, `main.rs` 2 passed, architecture suite 18 passed**. Inference: 71 passed at step 7 (crate unchanged, not re-run).

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

1. **Step 9: outbox (`0004_outbox`).** The design is approved (`docs/superpowers/specs/2026-09-30-outbox-design.md`, branch `refactor/outbox`) and the plan is `docs/superpowers/plans/2026-09-30-outbox.md`; implement it task by task (Sonnet implements, Opus reviews each task). Original brief: write the design, get the user's approval, then the plan (`SPEC.md` §5.3 and `docs/superpowers/plans/2026-09-28-eliminate-spec-gaps.md` outline it): `outbox_deliveries`, SSE fed by `LISTEN`/`NOTIFY`, remove `EventBroadcaster` and `webhook_outbox`. The webhook worker (`BotModule::spawn_background_workers`, `BackgroundWorkers`, `ModuleContext`) is the last user of the old worker mechanism and goes with it. Update `docs/architecture.md`, `docs/api.md` (the webhook queue is still described as unchanged) and `docs/database.md` in the same commits.
2. Clear the §11 items that touch the test suite first.
3. Hand the user the merge gates (§10). Merge only on their instruction.
4. Only then: sub-project 2 (Omnichannel), 3 and 4. Each starts with a design and the user's approval.

## 8. Feature template (established by Task 1, rooms)

Read the committed rooms code before starting any feature:
- `domain/rooms`
- `application/rooms`
- `infrastructure/postgres/repositories/rooms.rs`
- `interface/chat/rooms.rs`
- `application/mod.rs` (`AppServices`, `AppError`)

### Contract per feature `<f>`

| Layer | Path | Rules |
|---|---|---|
| Domain | `domain/<f>/mod.rs` | Pure entities, validated commands, invariants as functions. No `FromRow`, no `ToSchema`, no sqlx, no I/O. Its own `thiserror` error enum. |
| Service | `application/<f>/mod.rs` | `<F>Service { pool, events }` owns transactions (only where an invariant needs atomicity; keep the boundaries listed in the survey) and **publishes events after commit**. `From<FError> for AppError` (→ `Validation`). |
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
- **Wiring `AppServices`:** production builds it with the event broadcaster (`AppServices::with_events`). The constructor without events is test-only. Never let production silently drop events.
- **Many parameters:** prefer a small context struct over `#[allow(clippy::too_many_arguments)]`. `run_ai_turn`'s `#[allow]` becomes `AiTurnContext` in Task 13.
- **Tie-breaks:** among equal timestamps, the first item in picker order wins. Use a strict `>`; **never `max_by`**, which returns the *last* of equal maxima.
- **Behaviour is frozen.** If you find an existing bug, keep the behaviour and report it. If it is a security or data-correctness bug, fix it, test it, and disclose it in the commit body. Rooms did this for literal `%`/`_` matching; settings did it for validated writes.
- **Read-modify-write:** go through one service method that locks the row (`FOR UPDATE`) inside a transaction. Never fall back to defaults on a read error: only a missing row means defaults.
- **Room resolution never fails:** each step's read error is logged with `warn!` and treated as empty (`logged_or_default`).
- **Testing:**
  - Domain: plain `#[test]`.
  - Repository: `test_pool()` + `run_migrations_unlocked`.
  - Service: transaction atomicity (a NUL byte in a TEXT field forces a failure mid-transaction), degradation (a closed pool), events emitted only after commit.
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

## 10. User gates before merging sub-project 1

These need the human. Prepare them; don't perform them.

1. **Production rehearsal:** restore a `pg_dump` of production into a scratch DB and run `MIGRATE_TEST_DB=… cargo test migrates_a_real_database -- --ignored`. Order: run master once, then take the dump (see `docs/superpowers/runbooks/postgres-foundation-cutover.md`).
2. **Supervised first boot on matrix-sdk 0.18** against a **copy** of `data/`, following the watch list in the runbook.
3. **Stray device:** the user checks the bot account for a stray device from about 2026-09-25 00:24 local time.

## 11. Known deferred items

- Clippy is not yet a CI job (workspace is at zero warnings).
- `statement_timeout` is unset (R7).
- The AI repository has a TTL timezone issue.
- `SyncHealth`: if the timestamp fails to parse, it reads as "never synced" (theoretical).
- The AI tool `update_my_settings` advertises `dev_room_id` and `max_history_turns` but ignores them. This is an existing bug and needs a separate fix.
- Historical plan and review docs still mention the removed `/api/v1/health`. This is intentional.
- The architecture-test scanner cuts each line at `//`, so a URL literal hides the rest of the line. Task 14 makes it strip string literals.
- Resolved in the step 8 final fix wave: the `interface::http::auth::matrix` tests no longer reach the network (`test_state()` pre-fills `resolved_homeserver` with a closed loopback port and the client has a 500 ms timeout), so no test needs `--skip` any more.
- No handler test pins the portfolio snapshot capture call (R38): deleting `capture_current_snapshot_for_owner` from `RefreshPricesHandler` would pass the handler tests; only the application-layer test covers the write.
- ~~Test-pool schema leaks~~ — resolved by `5469098` (`db::test_schemas` reaps schemas of dead test processes), merged into this branch.
- Resolved in the final fix wave: the two flaky timing tests (`the_runtime_runs_jobs_and_stops_on_cancel` now waits `shutdown_grace` plus a margin and the reaper races each query against shutdown; the wake-up test waits for the listener's catch-up wake instead of sleeping, and the worker registers for wake-ups before it claims).
- `error.rs` and `events.rs` still sit at the crate root, outside the layer layout (the user flagged this as a file-structure issue); step 9 moves `events`, and `error.rs` needs a home.
- (Resolved: `AccountId::from_seed` is now `#[cfg(test)]`.)
- Portfolio price refreshes now run concurrently across workers (one `portfolio.refresh_prices` job per holding owner); the old loop refreshed owners one after another, so Yahoo load is no longer serialized across owners.
- The briefing window does not wrap the hour (`07:58` is due only at :58–:59). Frozen behaviour, kept.
- `!briefing now` still publishes `MorningBriefingSent` from the chat handler instead of `BriefingService::record_sent` (candidate cleanup).
- `BotModule::spawn_background_workers`, `BackgroundWorkers` and `ModuleContext` remain only for the webhook worker and go with step 9.
- Interface code still calls `repositories::mcp::McpTokenRepository` directly (`interface/chat/mcp.rs`, `interface/http/mcp.rs`, `interface/http/auth/mod.rs` tests): candidate for an `McpService`.
- `domain::accounts::StoredCredential::passkey` imports `webauthn_rs`/`serde_json`/`tracing`, outside the domain crate list (pre-existing).
- `domain/accounts/session.rs` imports `sha2`/`webauthn_rs`/`serde_json`/`tracing`, also outside the domain crate list (pre-existing purity breach).
- A recurring reminder is marked `failed` if the send succeeded but advancing it hit a database error on the last attempt.
- The HTTP recovery path reaches `interface::chat::redeem_matured_recovery`, whose `if let Ok` turns a database error into a 401.
- Pre-existing re-link merge offer in `repositories/accounts/linking.rs:129-159`: an MXID removed from account A can pull A's data into a new account. Needs its own ticket.
- `JOB_WORKERS` is not validated against `DATABASE_MAX_CONNECTIONS` (too many workers can starve the pool).
- `ReminderService::enqueue_due` is unbounded (one pass enqueues every due reminder).

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
| `docs/superpowers/runbooks/postgres-foundation-cutover.md` | Production cutover and rollback |
| `AGENTS.md` | Rules for agents: architecture, workflow, constraints, verification, Definition of Done |
| `docs/architecture.md` | Layers, one-capability-many-interfaces, Matrix/accounts/settings design rationale |
| `docs/database.md` | Schema and SQLx conventions, migrations, test databases |
| `docs/api.md` | REST/SSE conventions, authentication, webhooks, OpenAPI workflow |
| `docs/llm.md` | LLM transport, tool loop, agent limits, MCP |
| `README.md` | Setup, deployment, *Development* (toolchain, tests, Gitea and CI) |
| `docs/superpowers/plans/2026-09-28-eliminate-spec-gaps.md` | Master plan across all four sub-projects |
