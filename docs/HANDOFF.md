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
3. Set up the environment and start the test database (`docs/DEVELOPMENT.md`), then run the verification list (§5). It must be green before you change anything.
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
    channels/matrix/ matrix-sdk client, sync loop, E2EE, verification
  llm/ jobs/ events/ added by Task 13 and design steps 8–9
  modules/  db/executor.rs  db/dialect.rs   ← interim; deleted in Task 14
```

**Dependency rule** (enforced by `backend/tests/architecture.rs`):

| Layer | May not import |
|---|---|
| `domain` | anything except the pure crates listed in the tree above |
| `application` | axum, matrix-sdk, reqwest, utoipa, `interface` |
| `infrastructure` | `application`, `interface`, `modules` |

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
| 6 | Feature layering (Tasks 1–14 below) | ⏳ | — |
| 7 | Account UUID (`0002_account_uuid`) | ☐ needs a plan | — |
| 8 | Job queue (`0003_jobs`), then move the reminder, briefing, portfolio and bills loops onto it | ☐ needs a plan | — |
| 9 | Outbox (`0004_outbox`, `outbox_deliveries`, SSE via LISTEN; remove `EventBroadcaster` and `webhook_outbox`) | ☐ needs a plan | — |
| 10 | Docs: `AGENTS.md` (remove the macro and portability rules; add layer and migration rules) and `docs/ARCHITECTURE.md` | ☐ | — |

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
| 13 | ai + mcp (→ `llm/`, `AiTurnContext`, `MAX_TOOL_ROUNDS`) | ☐ |
| 14 | delete the `db_*!`/`tx_*!` macros and `modules/`; harden the architecture test | ☐ |
| — | whole-branch review + one fix wave | ☐ |

Remaining interim files that still run SQL from `interface/` (`INTERFACE_SQL_ALLOW_LIST`; the list can only shrink):
- `interface/channels/matrix/{invite,reaction_targets}.rs`
- `interface/http/{mcp,mod}.rs`

The last full test run: **803 passed / 0 failed / 13 ignored**.

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
cargo clippy -p matrix-bot --all-targets                 # touched files add no new warnings (102 pre-existing)
npm --prefix frontend run build                          # when anything frontend-facing changed
```

If a query changes, regenerate the cache with `cargo sqlx prepare --workspace -- -p matrix-bot --all-targets` and commit `.sqlx/`.

### Test database

```bash
docker run -d --name pg18-test --shm-size=1g -p 55433:5432 -e POSTGRES_PASSWORD=bot postgres:18
# TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres
# DATABASE_URL (for sqlx prepare) = same URL, after migrations have run once (any test run does it)
```

The tests create a schema per test and never drop it, which is why the container needs `--shm-size=1g`. To clean up, drop the leftover `test_%` schemas or recreate the container.

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

1. **Finish Task 2 (settings).** Review the fix diff `227c95c..ecef800` against these rulings:
   - every `UserSettings` write is validated in `SettingsService`, and the AI tool `update_my_settings` returns validation errors to the model;
   - `AppServices::new` (the constructor without events) is `#[cfg(test)]`, and production uses `AppServices::with_events`;
   - `SettingsService::modify` does `SELECT … FOR UPDATE` in one transaction, propagates read errors, publishes after commit, and every read-then-save caller uses it.

   Check for new defects: events published inside the transaction, lock ordering, validation that changes valid inputs, empty tests. Then mark Task 2 ✅ in §4.
2. **Task 3: accounts core + `AccountId`.**
   - Repoint `RoomService::linked_mxids` at the accounts service.
   - Thread `AccountId` through the rooms and settings services.
   - In the report, list every `AGENTS.md` Security Rule and the test that proves it.
3. Tasks 4–14 in plan order, then the whole-branch review and one fix wave.
4. Design, plan and implement steps 7 (UUID), 8 (jobs), 9 (outbox) and 10 (docs). Each design needs the user's approval.
5. Hand the user the merge gates (§10). Merge only on their instruction.
6. Only then: sub-project 2 (Omnichannel), 3 and 4. Each starts with a design and the user's approval.

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
- **R5:** the principles are folded into `AGENTS.md`. Clippy is stated as a rule but not enforced in CI (102-warning backlog).
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

## 10. User gates before merging sub-project 1

These need the human. Prepare them; don't perform them.

1. **Production rehearsal:** restore a `pg_dump` of production into a scratch DB and run `MIGRATE_TEST_DB=… cargo test migrates_a_real_database -- --ignored`. Order: run master once, then take the dump (see `docs/runbooks/postgres-foundation-cutover.md`).
2. **Supervised first boot on matrix-sdk 0.18** against a **copy** of `data/`, following the watch list in the runbook.
3. **Stray device:** the user checks the bot account for a stray device from about 2026-09-25 00:24 local time.

## 11. Known deferred items

- 102 pre-existing clippy warnings.
- `statement_timeout` is unset (R7).
- The AI repository has a TTL timezone issue.
- `SyncHealth`: if the timestamp fails to parse, it reads as "never synced" (theoretical).
- The AI tool `update_my_settings` advertises `dev_room_id` and `max_history_turns` but ignores them. This is an existing bug and needs a separate fix.
- Historical plan and review docs still mention the removed `/api/v1/health`. This is intentional.
- The architecture-test scanner cuts each line at `//`, so a URL literal hides the rest of the line. Task 14 makes it strip string literals.

## 12. Document map

| Path | What |
|---|---|
| `SPEC.md` | Roadmap for all six phases, the table inventory, NFRs |
| `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` | Binding design for sub-project 1 (D1–D10, §3 structure, §4 data, §5 jobs/outbox, §6 order) |
| `docs/superpowers/plans/2026-09-24-postgres-only-and-migrations.md` | Steps 1–4 (done) |
| `docs/superpowers/plans/2026-09-25-layer-skeleton.md` | Step 5 (done) |
| `docs/superpowers/plans/2026-09-25-feature-layering.md` | Step 6: Tasks 1–14, decisions D-A…D-H, the Feature Migration Contract |
| `docs/superpowers/plans/2026-09-25-feature-survey.md` | Per-feature facts: sizes, tables, coupling, transactions, adapters |
| `docs/runbooks/postgres-foundation-cutover.md` | Production cutover and rollback |
| `AGENTS.md` | Hard rules, Engineering Principles, Definition of Done, Security Rules |
| `docs/ARCHITECTURE.md` | Rationale behind the non-obvious design choices |
| `docs/DEVELOPMENT.md` | Environment setup, test database, every test suite, troubleshooting |
| `docs/GITEA.md` | Remotes, branches, pull requests, Gitea Actions CI, agent rules for the forge |
