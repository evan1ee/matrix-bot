# Handoff — PostgreSQL Foundation Refactor

**Read this first if you are an agent taking over this refactor.** It says where the work stands, how it is being done, what comes next, and which decisions are already settled. Don't reopen a settled decision unless you have new evidence.

- **Branch:** `refactor/postgres-layered`. It was cut from `refactor/postgres-foundation` at `ecef800` and holds all the work so far: 35 commits on top of `master` (`81d9c03`) plus this document.
- **Status (2026-09-27):** design steps 1–5 are done. Step 6 (feature layering) is in progress: **Task 1 (rooms) is complete. Task 2 (settings) is implemented and fixed but still awaiting its final re-review.** Steps 7–10 are not started.
- **Not merged, not pushed.** Merging and pushing both need the user's explicit approval, and the user gates in §6 must be passed first.

---

## 1. Source documents (on this branch)

| Document | Role |
|---|---|
| `SPEC.md` (v2.4.0) | Product and architecture roadmap. Covers the §4 table inventory, the outbox with `outbox_deliveries`, jobs with `dedupe_key` and heartbeat, the tool-naming rule `<ns>_<action>`, `MAX_TOOL_ROUNDS = 5`, the §13 layer-first tree, and PostgreSQL 18+. |
| `docs/superpowers/specs/2026-09-24-postgres-foundation-design.md` | **The binding design.** Decisions D1–D10, the target structure (§3), the data layer (§4), jobs and outbox (§5), and the **order of work (§6)**. |
| `docs/superpowers/plans/2026-09-24-postgres-only-and-migrations.md` | Plan for design steps 1–4. **Done.** |
| `docs/superpowers/plans/2026-09-25-layer-skeleton.md` | Plan for design step 5. **Done.** |
| `docs/superpowers/plans/2026-09-25-feature-layering.md` | Plan for design step 6: Tasks 1–14, Decisions D-A…D-H and the **Feature Migration Contract**. **In progress.** |
| `docs/superpowers/plans/2026-09-25-feature-survey.md` | Per-feature facts: sizes, macro counts, tables, coupling, transactions, adapters. Read the section for your feature before you start a task. |
| `docs/runbooks/postgres-foundation-cutover.md` | Production cutover steps and rollback. |
| `AGENTS.md` | Hard project rules: Engineering Principles, Definition of Done, Module Implementation Checklist (now the Feature Migration Contract), Security Rules, Gotchas. |

## 2. User decisions (settled)

Collected by interview; don't reopen them.

- **Scope:** foundation first (roadmap phases 1–3). PostgreSQL is the only application database; there is no SQLite support. matrix-sdk keeps its own SQLite crypto store.
- **Data:** adopt the live PostgreSQL in place. `0001_baseline.sql` is idempotent and matches the legacy schema.
- **IDs:** BIGINT row IDs; accounts get UUIDs in a later migration (`0002_account_uuid`).
- **Queries:** compile-checked `sqlx::query!`/`query_as!`/`query_scalar!` with the offline `.sqlx/` cache. `SQLX_OFFLINE=true` is set in `.cargo/config.toml`.
- **Versions:** PostgreSQL **18**, sqlx **0.9**, matrix-sdk **0.18**. They were upgraded together because of Cargo's `links = "sqlite3"` conflict.
- **Layout:** layer-first, per spec §13 and design §3:
  - `domain/`, `application/`, `interface/{http,chat,channels/matrix}`, `infrastructure/{inference,postgres}`, plus `llm/`, `jobs/` and `events/` later.
  - `app.rs` is the composition root.
- **Topology:** single instance, ready for high availability later (advisory locks, `SKIP LOCKED`).
- **Outbox:** every event goes through it, with one delivery row per destination (`outbox_deliveries`).
- **Job queue:** hand-rolled (`0003_jobs`: `SKIP LOCKED`, `dedupe_key`, heartbeat reaper, advisory-lock scheduler).
- **Engineering principles:** the user's Core Engineering Principles are merged into `AGENTS.md`. Follow them.

## 3. Progress

### Done (design steps 1–5)

| Step | Commits (oldest→newest) |
|---|---|
| 1. PostgreSQL-only cut | `382d1dd` `6c3d384` `f7dec54` `07897ab` |
| 2. matrix-sdk 0.18 + sqlx 0.9 | `f4ef60a` `c8576b6` |
| 3. Versioned migrations (`sqlx::migrate!`, `0001_baseline.sql`) | `051444b` `68dfc56` `b803bef` (LF pinning) |
| 4. Compile-checked query pipeline and hardening | `0bd18c0` `9c2865f` `e9fe149` `a52b984` `1af27e8` `4975039` |
| 4. Docs: engineering principles, runbook | `b502e2a` `e85d89b` `71ec65e` `1d139eb` `c6d304b` |
| 5. Layer skeleton | `e208254` `fbd5252` `3b53386` `85c2dbc` `e4f1aab` |
| 5. Layer skeleton fix wave | `778585e` `45020fe` `ac816e5` `be24444` |

What exists now:
- **Composition root:** `app.rs` holds one `CancellationToken` tree, bounds shutdown at `SHUTDOWN_GRACE` = 15 s, and supervises both front doors (the HTTP API and the Matrix channel). If either one dies, the process exits.
- **Health probes:**
  - `/health/live`.
  - `/health/ready`: a 2 s DB ping, plus a check that the last Matrix sync is less than 120 s old.
- **Architecture test** (`backend/tests/architecture.rs`) enforces:
  - the dependency rule;
  - no `dotenvy::dotenv` anywhere;
  - a shrinking `INTERFACE_SQL_ALLOW_LIST`;
  - that every scanned directory exists.
- **`.env` loading:** only from the current directory (`main.rs` → `load_dotenv_from`), never from a parent directory.
- **Docker and dev:** `docker-compose.yml` sets `stop_grace_period: 20s`, the Dockerfile copies `.sqlx`, and Vite proxies both `/api` and `/health`.

### Step 6: feature layering (`2026-09-25-feature-layering.md`)

| Task | Scope | State |
|---|---|---|
| 1 | rooms (template) | ✅ `14c74ab` + fix `60d14b3` (reviewed) |
| 2 | settings | ⏳ `227c95c` + fix round 1 `ecef800`. **The scoped re-review of `ecef800` was cut off by a rate limit. Run it first** (see §5). |
| 3 | accounts core + `AccountId` | not started |
| 4 | accounts merge (D-A) | not started |
| 5 | bookmarks | not started |
| 6 | footprints + habits | not started |
| 7 | webhook | not started |
| 8 | todo + reminder + recurring_bills + due_reminder (D-E, D-H) | not started |
| 9 | finance | not started |
| 10 | portfolio | not started |
| 11 | life + journal + briefing (read models) | not started |
| 12 | ocr + voice + tts + core + account command module | not started |
| 13 | ai + mcp (→ `llm/`, `AiTurnContext`) | not started |
| 14 | remove the macros and `modules/`, harden the architecture test | not started |

After Task 14 comes the plan's final whole-branch review and one fix wave.

### Steps 7–10: not planned yet

Each step needs its own plan in `docs/superpowers/plans/`, written with the `superpowers:writing-plans` skill. Each plan is then run through `superpowers:subagent-driven-development`.

- **7. Account UUID** (`0002_account_uuid`): `owner_id` TEXT → UUID, with foreign keys, the `AccountId` type and a rehearsal test. It must come after step 6: once every query is compile-checked, `cargo check` lists every call site the type change touches.
- **8. Job queue** (`0003_jobs`): queue, worker, reaper and advisory-lock scheduler. Then move the reminder, briefing, portfolio and bills loops onto it.
- **9. Outbox** (`0004_outbox`): the dispatcher and `outbox_deliveries`, SSE via `LISTEN`/`NOTIFY`, and webhook deliveries. This removes `EventBroadcaster` and `webhook_outbox`.
- **10. Docs:** `AGENTS.md` (drop the portability tables and the macro rules; add layer rules, `cargo sqlx prepare` and migration rules) and `docs/ARCHITECTURE.md`.

## 4. How the work is done (process)

The process is **Superpowers subagent-driven development (SDD)**. For each task:

1. Record the base commit (BASE = `git rev-parse HEAD`).
2. Extract the task's text into a brief with the `task-brief` script from the `subagent-driven-development` skill. Dispatch **one** implementer subagent with the brief path and a report path. Use a standard model; the tasks are multi-file.
3. The implementer follows TDD, runs the full verification (below), commits, and writes a report.
4. Build the review package (`scripts/review-package <plan> BASE HEAD`) and dispatch a task reviewer. It judges spec compliance **and** code quality, and checks: no OpenAPI diff, conformance to the patterns in §7, and no silent behaviour change.
5. Fix rounds:
   - Rounds 1–3: resume the same implementer.
   - Rounds 4–5: use a fresh implementer on a stronger model.
   - After each round, run a scoped re-review of only the fix diff.
6. Record every decision in the ledger as `Ruling: <decision> — <why> — <cost if wrong>`, and keep going. Stop only for irreversible, security-sensitive or outward-facing actions: merge, push, production data, a live Matrix login.

**Ledger location:** `.superpowers/sdd/<plan-basename>/progress.md`. It is git-ignored. **The earlier ledgers were lost when their worktree was deleted.** §8 of this document replaces them. Start a fresh ledger, and seed it with a first line naming the plan plus "Tasks 1: complete; Task 2: re-review pending".

### Verification: every task must pass all of these

```bash
cargo fmt --all -- --check
cargo check -p matrix-bot --all-targets                 # warning-free
cargo test  -p matrix-bot --all-targets                 # last run: 808 passed / 0 failed / 11 ignored
cargo test  -p matrix-bot --test architecture
cargo test  -p matrix-bot live_postgres -- --ignored --test-threads=1
cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets   # against a migrated scratch DB
cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json   # must be NO diff
# clippy: touched files add no new warnings (102-warning backlog is pre-existing; not CI-enforced)
```

**Test database** (local Docker, recreated fresh; start it if it is gone):

```bash
docker run -d --name pg18-test --shm-size=1g -p 55433:5432 -e POSTGRES_PASSWORD=bot postgres:18
# TEST_DATABASE_URL / DATABASE_URL for sqlx prepare: postgres://postgres:bot@localhost:55433/postgres
```

`--shm-size=1g` is required. The tests create a schema per test and never drop it, so shared memory eventually runs out. Clean up between long sessions by dropping the leftover `test_%` schemas, or recreate the container.

You can set `CARGO_TARGET_DIR=D:/01_Data/code/matrix-hello-bot/target` to reuse the main checkout's build cache.

### Safety constraints (hard)

- **Never read or modify the main checkout's `.env`**: it holds production credentials, and may point at production PostgreSQL.
- **Never read or modify the main checkout's `data/`**: it holds the real Matrix crypto store.
- **Never run the bot binary.** The only exception is `cargo run --bin generate-openapi`. Never contact a homeserver or any other external service.
  - An earlier boot check picked up the main `.env` through dotenvy's upward search and started a real Matrix login. That is why `.env` is now loaded only from the current directory, and why the architecture test bans `dotenvy::dotenv`. Only `dotenvy::from_path` is allowed.
- **Never push, merge or open a PR** without the user saying so. When the user does, Gitea (`git.mbm.one`) is the primary forge and GitHub (`origin`) is a mirror.
- **Never use bare `git stash`.** The stash stack is shared across worktrees.

## 5. Next actions, in order

1. **Re-review Task 2 fix round 1** (diff `227c95c..ecef800`) against these rulings:
   - **I1:** every `UserSettings` write goes through `SettingsService` validation, including the AI tool `update_my_settings`, which returns the validation error to the model.
   - **I2:** `AppServices::new` (the constructor with no events) is `#[cfg(test)]`; production uses `AppServices::with_events`.
   - **I3:** `SettingsService::modify` does `SELECT … FOR UPDATE` in one transaction, propagates read errors, publishes after commit, and every read-then-save caller goes through it.

   Also check for new defects: an event published inside the transaction, lock ordering, validation that changes behaviour for valid inputs, and tests that assert nothing. The original review listed M1–M8 as minor findings; the implementer says all were fixed, but the review file was lost, so judge the fix diff on its own merits. If it's clean, mark Task 2 complete.
2. **Task 3: accounts core + `AccountId`.** Carry these forward:
   - `RoomService::linked_mxids` must be repointed at the accounts service.
   - List every AGENTS.md Security Rule and the test that proves it.
   - Mechanically thread `AccountId` through the rooms and settings services.
3. Tasks 4–14 in plan order, then the final whole-branch review and one fix wave.
4. Write and execute the plans for design steps 7, 8, 9 and 10.
5. Stop for the user gates in §6 before any merge.

## 6. User gates (pending; need the human)

- **Production rehearsal (R1):** restore a `pg_dump` of production into a scratch DB and run `MIGRATE_TEST_DB=… cargo test migrates_a_real_database -- --ignored`. The runbook has the exact order: run master once before the branch, then take the dump.
- **Supervised first boot on matrix-sdk 0.18** against a **copy** of `data/`. The first-boot watch list is in the runbook. Implementers must never do this themselves: a second client on the bot's device can consume one-time keys.
- **Stray device check:** the user should look for a stray device on the bot account from about 2026-09-25 00:24 local time, left by the accidental login in §4.

## 7. Feature template (patterns from Task 1: rooms)

Read the committed rooms code first:
- `domain/rooms`
- `application/rooms`
- `infrastructure/postgres/repositories/rooms.rs`
- `interface/chat/rooms.rs`
- `app.rs`: `AppServices`

**Contract.** The full text is in the plan and in AGENTS.md.

| Layer | Rules |
|---|---|
| Domain | Pure types. No `FromRow`, no `ToSchema`, no sqlx. |
| Service | A `<F>Service` owns its transactions and publishes events **after commit**. |
| Repository | Free functions over `impl sqlx::PgExecutor<'_>`, with private `Row` structs and a `From`/`TryFrom` into the domain type. |
| HTTP | DTOs keep the exact current schema, so `openapi.json` stays byte-identical. |
| Chat | Handlers live in `interface/chat/<f>.rs`. `BotModule::handle_command` and `ModuleRegistry::dispatch` take `&AppServices`. Use `interface::chat::error_reply` for errors and `AppError::External { service, source }` for failures from external services. |

Each task also:
- adds its service to `AppServices`;
- removes its files from `INTERFACE_SQL_ALLOW_LIST`;
- repoints its AI and MCP tools at the new service.

**Decisions that carry forward:**
- **Service parameter position:**
  - `handle_command`: `services` comes after `ctx`.
  - AI, tool and MCP functions: `services` is the first parameter.
  - Moved code reaches services through these parameters and never adds new ones.
- **Constructing `AppServices`:** production builds it with the event broadcaster (`AppServices::with_events`); the constructor without events is test-only. Never add a production path that silently drops events.
- **Many-argument functions:** prefer a small context struct over `#[allow(clippy::too_many_arguments)]` when threading services through them. `run_ai_turn`'s `#[allow]` becomes `AiTurnContext` in Task 13.
- **Errors:** there is no crate-wide `DomainError`. Each feature has its own `thiserror` enum in `domain/<f>` and a `From<FError> for AppError` (→ `Validation`) in `application/<f>`.
- **`From` vs `TryFrom`:** use `From` for Row → domain conversions that cannot fail, `TryFrom` otherwise.
- **Tie-breaks:** among equal timestamps, the first item in picker order wins. Use a strict `>` comparison; **never `max_by`**, which returns the *last* of equal maxima.
- **Behaviour must not change.** If you find an existing bug, keep the behaviour and report it. The exception is a security or data-correctness bug: fix it, test it, and disclose it in the commit body and the report. Rooms did this for literal `%`/`_` matching in room names; settings did it for unvalidated AI writes.
- **Room resolution never fails.** Per-step read errors are logged with `warn!` and treated as empty, in `logged_or_default`.
- **Testing:**

  | Layer | How |
  |---|---|
  | Domain | Plain `#[test]` |
  | Repository | `test_pool()` + `run_migrations_unlocked` |
  | Service | Transaction atomicity (a NUL byte in a TEXT field forces a failure mid-transaction) and degradation (a closed pool) |
  | Handlers | Existing tests move with their handler |

## 8. Rulings made so far (replaces the lost ledgers)

**Foundation (steps 1–4)**
- **R1:** the production rehearsal is a release gate at finish, run with the user.
- **R2:** the target is PostgreSQL 18 (user directive).
- **R3/R4:** the target is sqlx 0.9, which forces matrix-sdk 0.18 (newest version compatible with `libsqlite3-sys` 0.35). The two upgrades were merged into one commit (`c8576b6`) because neither builds alone.
- **Task 3a:** no live Matrix boot by the implementer; it is a supervised user gate.
- **matrix-sdk 0.18 voice replies:** accepted the change to TTS replies. They now render as an MSC3245 voice bubble, which matches the documented invariant.
- **R5:** the engineering principles are folded into AGENTS.md. Clippy is stated as a rule but not CI-enforced, because of a 102-warning backlog.
- **CI:** `sqlx-cli` is cached.
- **R6:** the dotenv upward search is removed. This was a security fix that had to land before merge.
- **R7:** `statement_timeout` is deferred and recorded in the spec; revisit it with the job queue.
- **R8:** the test-schema leak is handled in the docs (`--shm-size=1g` plus a cleanup command), not in code.
- **R9/R10:** the pre-merge residuals were fixed on this branch, and the work continues here through all the design steps. The user asked to remove the test container; it was recreated fresh.

**Feature layering (step 6)**
- **R11 (D-A):** merge's dynamic SQL is allowed only in `infrastructure/postgres/merge.rs`, and `AssertSqlSafe` is allowed nowhere else.
- **R12 (D-B):** the HTTP contract is frozen, so `openapi.json` must be byte-identical. Domain types derive neither `FromRow` nor `ToSchema`.
- **R13 (D-C/D-D/D-E):**
  - Matrix handlers go to `interface/chat/<f>.rs`.
  - Cross-feature reads go only to `infrastructure/postgres/read_models`.
  - todo, reminder, recurring_bills and due_reminder move as one batch.
- **Layer skeleton Task 4:** a failure to bind the API port now aborts boot (fail fast).
- **Rooms:**
  - Room targets containing `%` or `_` are matched literally. This is a bug fix: such a target used to match an arbitrary room of the user.
  - The first-in-order tie-break is kept.
  - `&AppServices` is threaded through `handle_command`, `dispatch` and the AI entry points (`run_ai_turn*`, `handle_ai_command*`, `ToolRegistry::execute_tool`, `executor::execute`, `build_system_prompt`).
- **Settings:**
  - `UserSettings` keeps `pub` fields, but its only persistence path validates.
  - The constructor without events is test-only.
  - Read-modify-write goes through `SettingsService::modify` (`FOR UPDATE`, one transaction).

## 9. Known deferred items and follow-ups

- **Clippy backlog:** 102 pre-existing warnings.
- **`statement_timeout`:** unset (R7).
- **AI repository:** a TTL timezone issue.
- **`SyncHealth`:** if the timestamp fails to parse (`DateTime::from_timestamp_millis` fails), it reads as "never synced". This is theoretical.
- **Existing bug:** the AI tool `update_my_settings` advertises `dev_room_id` and `max_history_turns` but ignores them. It needs a separate fix.
- **Stale references:** historical plan and review docs still mention the removed `/api/v1/health`. This is intentional.
- **Architecture-test scanner:** it cuts each line at `//`, so a URL literal hides the rest of that line. Task 14 fixes this by stripping string literals.
