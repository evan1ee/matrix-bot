# Layer Skeleton Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the backend its layer-first shape — composition root, layer roots, the three directory moves, liveness/readiness probes and bounded graceful shutdown — without changing any feature behaviour.

**Architecture:** Move `api/` → `interface/http/`, `bot/` → `interface/channels/matrix/`, `services/` → `infrastructure/inference/` as pure path moves; add documented `domain/` and `application/` roots guarded by a source-scan architecture test; replace the `broadcast::<()>` shutdown with one `tokio_util::sync::CancellationToken` tree owned by a new `app.rs`, bounded at 15 s; add `/health/live` and `/health/ready` (database + Matrix sync freshness) and retire `/api/v1/health`.

**Tech Stack:** Rust 2021, axum 0.7, tokio, tokio-util (new direct dependency, already transitive), sqlx 0.9 (PostgreSQL 18), matrix-sdk 0.18, utoipa, React frontend (`frontend/`).

**Spec:** [`docs/superpowers/specs/2026-09-24-postgres-foundation-design.md`](../specs/2026-09-24-postgres-foundation-design.md) §3 (structure, dependency rule), §3.1, §5.3 (operations), §6 step 4. Project rules: `AGENTS.md` (Engineering Principles, Definition of Done).

## Global Constraints

- No feature behaviour changes. Every existing test keeps passing; test count may only grow.
- Moves are `git mv` so history follows; no file content changes in a move commit other than `crate::…` path rewrites and doc-comment path mentions.
- Dependency rule (spec §3.1): `domain` depends on nothing but std/chrono/uuid/serde/thiserror; `application` never imports axum, matrix-sdk or reqwest; `interface` never calls `sqlx::query*`/`db_*!` directly (enforced for new code; existing `interface/http` handlers that already do are listed in an allow-list the test prints, to be emptied by the per-feature plan).
- Shutdown bound: 15 s total from signal to process exit (spec §5.3).
- Readiness: 200 when `SELECT 1` succeeds **and** the Matrix sync loop completed a sync within the last 120 s; otherwise 503 with a JSON reason (spec §5.3).
- Never run the bot binary against real services. `main.rs` reads `./.env` only; tests that need boot behaviour use in-process routers.
- Every task ends with: `cargo fmt --all -- --check`; `cargo check -p matrix-bot --all-targets` warning-free; `cargo test -p matrix-bot --all-targets`; `cargo test -p matrix-bot live_postgres -- --ignored --test-threads=1`; `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets` against a migrated scratch DB; `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json` (regenerate + commit when the task changes the API); clippy on changed files adds no new warnings.
- Local test DB: `TEST_DATABASE_URL=postgres://postgres:bot@localhost:55433/postgres` (container `bot-pg18-test`), `CARGO_TARGET_DIR=D:/01_Data/code/matrix-hello-bot/target`.
- Conventional commits with the session's co-author trailer.

---

## File Map

| Path | Change |
|---|---|
| `backend/src/api/**` → `backend/src/interface/http/**` | Task 1 move |
| `backend/src/bot/**` → `backend/src/interface/channels/matrix/**` | Task 1 move |
| `backend/src/services/**` → `backend/src/infrastructure/inference/**` | Task 1 move |
| `backend/src/interface/mod.rs`, `interface/channels/mod.rs`, `infrastructure/mod.rs` | Task 1 create (module roots with doc comments) |
| `backend/src/lib.rs` | Tasks 1, 2 |
| `backend/src/bin/generate_openapi.rs` | Task 1 path |
| `backend/src/domain/mod.rs`, `backend/src/application/mod.rs` | Task 2 create |
| `backend/tests/architecture.rs` | Task 2 create |
| `backend/src/interface/channels/matrix/sync_health.rs` | Task 3 create |
| `backend/src/interface/http/health.rs`, `routes.rs`, `mod.rs`, `openapi.rs` | Task 3 |
| `docker-compose.yml`, `Dockerfile`, `frontend/src/…` health caller, `frontend/e2e/fake-api.ts` | Task 3 |
| `backend/src/app.rs` | Task 4 create |
| `backend/src/main.rs`, `backend/src/interface/http/mod.rs` (`start_api_server`) | Task 4 |
| `backend/Cargo.toml` | Task 4 (`tokio-util`) |
| `AGENTS.md`, `docs/ARCHITECTURE.md`, `README.md` | every task, for the paths/behaviour it changes |

---

### Task 1: Move api/, bot/, services/ into the layer layout

**Files:** see File Map rows for Task 1.

**Interfaces:**
- Produces: modules `crate::interface::http` (was `crate::api`), `crate::interface::channels::matrix` (was `crate::bot`), `crate::infrastructure::inference` (was `crate::services`). `lib.rs` exports `pub mod interface; pub mod infrastructure;` and no longer `api`, `bot`, `services`.

- [ ] **Step 1: Create the module roots**

`backend/src/interface/mod.rs`:
```rust
//! Everything that talks to the outside world on the application's behalf:
//! the HTTP API (`http`), chat channels (`channels`). Interface code
//! translates transport payloads into application calls and back; it holds
//! no business rules and runs no SQL of its own (see `tests/architecture.rs`).

pub mod channels;
pub mod http;
```
`backend/src/interface/channels/mod.rs`:
```rust
//! Chat platforms. Matrix is the only one today; Discord and Telegram arrive
//! as siblings behind the `ChannelAdapter` trait in the omnichannel phase.

pub mod matrix;
```
`backend/src/infrastructure/mod.rs`:
```rust
//! Adapters to external systems: the inference/LLM HTTP clients (`inference`).
//! PostgreSQL repositories move here feature by feature.

pub mod inference;
```

- [ ] **Step 2: Move with git**

```bash
git mv backend/src/api backend/src/interface/http
git mv backend/src/bot backend/src/interface/channels/matrix
git mv backend/src/services backend/src/infrastructure/inference
```
Each moved directory's `mod.rs` stays its module root.

- [ ] **Step 3: Rewrite paths**

```bash
grep -rl "crate::api\b\|crate::api::" backend/src | xargs sed -i 's/crate::api::/crate::interface::http::/g; s/crate::api\b/crate::interface::http/g'
grep -rl "crate::bot::" backend/src | xargs sed -i 's/crate::bot::/crate::interface::channels::matrix::/g'
grep -rl "crate::services::" backend/src | xargs sed -i 's/crate::services::/crate::infrastructure::inference::/g'
sed -i 's/matrix_bot::api::/matrix_bot::interface::http::/g' backend/src/bin/generate_openapi.rs
```
Then fix what the compiler reports: `use super::…` inside moved trees is unchanged; `use crate::{api, bot}`-style grouped imports need hand edits; `lib.rs` module list: remove `api`, `bot`, `services`, add `infrastructure`, `interface` (alphabetical). Run `cargo fmt --all`.

- [ ] **Step 4: Fix hard-coded source paths**

`interface/http/openapi.rs` `test_every_route_is_documented` reads `"src/api/routes.rs"` and `"src/api/mcp.rs"` → `"src/interface/http/routes.rs"`, `"src/interface/http/mcp.rs"`. Grep for any other string path to the old dirs: `grep -rn '"src/\(api\|bot\|services\)/' backend/src`. The existing test `modules/mod.rs::test_no_module_depends_on_the_http_layer` scans for `crate::api` — change the needle to `crate::interface::http` and keep its intent.

- [ ] **Step 5: Verify nothing else changed**

Run the global verification list. `git diff -M --stat HEAD` must show the moves as renames (similarity ≥ 90% for nearly every file). `grep -rn "crate::api\|crate::bot\|crate::services\|matrix_bot::api" backend/src` → no hits.

- [ ] **Step 6: Docs**

Update every path mention in AGENTS.md (`backend/src/api/…`, `backend/src/bot/…`, `services/` boundary rule — now `infrastructure/inference/` never imports from `modules/`), `docs/ARCHITECTURE.md`, `README.md` (the `bot/dispatcher.rs` link). The rule "`services/` never imports from `modules/`" becomes "`infrastructure/` never imports from `modules/`, `interface/` or `application/`" — keep it true (check with grep).

- [ ] **Step 7: Commit**

```bash
git add -A backend/src AGENTS.md docs/ARCHITECTURE.md README.md
git commit -m "refactor: move api, bot and services into interface/ and infrastructure/"
```

---

### Task 2: Domain and application roots with an enforced dependency rule

**Files:** Create `backend/src/domain/mod.rs`, `backend/src/application/mod.rs`, `backend/tests/architecture.rs`; modify `backend/src/lib.rs`.

**Interfaces:**
- Consumes: Task 1's module layout.
- Produces: `crate::domain` and `crate::application` (empty, documented); `tests/architecture.rs` that later plans extend by moving code into these roots.

- [ ] **Step 1: Write the failing architecture test**

`backend/tests/architecture.rs`:
```rust
//! Source-level guard for the dependency rule in
//! docs/superpowers/specs/2026-09-24-postgres-foundation-design.md §3.1.
//! A scan, not a compiler guarantee: cheap, and it catches the regressions
//! that matter (an adapter type leaking into domain, SQL in a handler).

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

fn src(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join(sub)
}

/// Every `(file, line)` in `dir` whose non-comment text contains a forbidden needle.
fn violations(dir: &str, forbidden: &[&str]) -> Vec<String> {
    let mut found = Vec::new();
    for file in rust_files(&src(dir)) {
        let text = fs::read_to_string(&file).expect("readable source file");
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if forbidden.iter().any(|needle| code.contains(needle)) {
                found.push(format!("{}:{}: {}", file.display(), n + 1, line.trim()));
            }
        }
    }
    found
}

#[test]
fn domain_depends_on_nothing_outside_itself() {
    let v = violations(
        "domain",
        &[
            "sqlx", "axum", "matrix_sdk", "reqwest", "utoipa",
            "crate::application", "crate::interface", "crate::infrastructure",
            "crate::modules", "crate::db",
        ],
    );
    assert!(v.is_empty(), "domain/ must stay pure:\n{}", v.join("\n"));
}

#[test]
fn application_does_not_know_transports() {
    let v = violations(
        "application",
        &["axum", "matrix_sdk", "reqwest", "utoipa", "crate::interface"],
    );
    assert!(v.is_empty(), "application/ must not depend on transports:\n{}", v.join("\n"));
}

#[test]
fn infrastructure_does_not_reach_upwards() {
    let v = violations(
        "infrastructure",
        &["crate::interface", "crate::application", "crate::modules"],
    );
    assert!(v.is_empty(), "infrastructure/ must not import upper layers:\n{}", v.join("\n"));
}
```
Register nothing in `lib.rs` yet and run it.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p matrix-bot --test architecture`
Expected: the `domain`/`application` tests pass vacuously (no dirs) — so first add a **RED check**: temporarily create `backend/src/domain/mod.rs` containing `use sqlx::PgPool;` and confirm `domain_depends_on_nothing_outside_itself` FAILS naming that line; record the output; then continue. (`infrastructure_does_not_reach_upwards` may already fail if Task 1 left an upward import — if so, fix the import at its source, don't weaken the test.)

- [ ] **Step 3: Create the roots**

`backend/src/domain/mod.rs`:
```rust
//! Business entities, value objects and invariants — pure Rust.
//!
//! Depends on nothing but std, chrono, uuid, serde and thiserror
//! (`tests/architecture.rs`). Features move here one at a time from
//! `modules/<feature>/models.rs` as the per-feature plan reaches them.
```
`backend/src/application/mod.rs`:
```rust
//! Use cases. A service method is the unit of transaction: it validates a
//! command through the domain, opens the transaction, calls repositories,
//! publishes the resulting event and commits. HTTP, Matrix, MCP and LLM
//! tools all call the same method. Never imports a transport
//! (`tests/architecture.rs`).
```
Add `pub mod application; pub mod domain;` to `lib.rs` (alphabetical).

- [ ] **Step 4: Verify** — the global list; `cargo test -p matrix-bot --test architecture` passes.

- [ ] **Step 5: Docs** — AGENTS.md: the Architecture/Engineering Principles text says the dependency rule is enforced by `backend/tests/architecture.rs`; list the test command under Commands. Remove "(approved, not yet built)" wording only for the parts that now exist.

- [ ] **Step 6: Commit** — `feat: add domain and application roots with an architecture test`

---

### Task 3: Liveness and readiness probes

**Files:** Create `backend/src/interface/channels/matrix/sync_health.rs`; modify `interface/channels/matrix/mod.rs` (+ wherever the sync loop records a completed sync), `interface/channels/matrix/handles.rs` (carry the health handle), `interface/http/health.rs`, `routes.rs`, `mod.rs` (mount at the router root, not under `/api/v1`), `openapi.rs`; `docker-compose.yml`, `Dockerfile` (healthcheck URLs); the frontend caller of `/api/v1/health` and `frontend/e2e/fake-api.ts`; regenerate `backend/openapi.json` and the frontend API types (`npm --prefix frontend run generate-api`).

**Interfaces:**
- Produces:
```rust
/// When the Matrix sync loop last completed a sync. Written by the sync loop,
/// read by `/health/ready`. Cheap to clone; lock-free.
#[derive(Clone, Default, Debug)]
pub struct SyncHealth(std::sync::Arc<std::sync::atomic::AtomicI64>); // unix millis; 0 = never

impl SyncHealth {
    pub fn record_sync(&self, at: chrono::DateTime<chrono::Utc>);
    pub fn last_sync(&self) -> Option<chrono::DateTime<chrono::Utc>>;
    /// Pure: whether the last sync is within `max_age` of `now`.
    pub fn is_fresh(&self, now: chrono::DateTime<chrono::Utc>, max_age: chrono::Duration) -> bool;
}
```
- `GET /health/live` → `200 {"status":"live"}` always, touches nothing.
- `GET /health/ready` → `200` or `503` with body
```json
{ "status": "ready" | "not_ready",
  "checks": { "database": { "ok": true },
              "matrix_sync": { "ok": true, "last_sync_at": "2026-09-25T10:00:00Z" } } }
```
`MATRIX_SYNC_MAX_AGE` = 120 s (a `const` in `health.rs`).

- [ ] **Step 1: Failing unit tests for `SyncHealth`** (`sync_health.rs` test module): never synced → `last_sync()` is `None`, `is_fresh` false; record at T → `last_sync()` == T (millisecond precision); fresh at T+119 s, stale at T+121 s; clones share state.
- [ ] **Step 2: Implement `SyncHealth`**; run the tests green.
- [ ] **Step 3: Record syncs.** Find where the Matrix loop observes a completed sync (the `sync`/`sync_with_callback` loop in `interface/channels/matrix/` — the implementer locates it) and call `record_sync(Utc::now())` after each successful sync response. Carry `SyncHealth` in `BotHandles` (shared with the API state the same way `BotHandles` already is).
- [ ] **Step 4: Failing router tests** in `health.rs` (in-process `axum` router via `tower::ServiceExt::oneshot`, pool from `test_pool()` + `run_migrations_unlocked`): `/health/live` 200; `/health/ready` 503 with `matrix_sync.ok=false` when never synced; 200 after `record_sync(Utc::now())`; 503 with `database.ok=false` when the pool is closed (`pool.close().await` before the request). No auth required on either route; neither is under `/api/v1`.
- [ ] **Step 5: Implement** the two handlers with `#[utoipa::path]`, mount at the router root, delete the old `health_check` and its `/api/v1/health` route and `HealthResponse`; `query_scalar!("SELECT 1 AS \"ok!\"")` stays the DB check (regenerate `.sqlx/` if the query text changes).
- [ ] **Step 6: Consumers (public contract change, intentional):**
  - `docker-compose.yml` and `Dockerfile` healthchecks → `/health/live` (a container restart cannot fix a homeserver outage, so liveness, not readiness, drives restarts; say so in a comment).
  - Frontend: the `core` module's health call → `/health/ready`, adapting to the new body; `frontend/e2e/fake-api.ts` route updated; `npm --prefix frontend run build` passes; `npm --prefix frontend run test:e2e` if Playwright is installed (report if not).
  - `openapi.json` regenerated and committed; frontend API types regenerated.
- [ ] **Step 7: Docs** — AGENTS.md / ARCHITECTURE.md / README health references.
- [ ] **Step 8: Verify + commit** — `feat: add liveness and readiness probes`

---

### Task 4: Composition root and bounded graceful shutdown

**Files:** Create `backend/src/app.rs`; modify `backend/src/main.rs`, `backend/src/lib.rs`, `backend/src/interface/http/mod.rs` (`start_api_server`), the Matrix loop's shutdown plumbing (`run_bot_loop` moves from `main.rs` into `app.rs` or `interface/channels/matrix/`), `backend/Cargo.toml` (`tokio-util = "0.7"` — match the version already in Cargo.lock).

**Interfaces:**
- Produces:
```rust
// app.rs
pub const SHUTDOWN_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

/// Builds every long-lived component, starts the API server and the Matrix
/// loop, waits for `shutdown` to be cancelled, then stops everything within
/// SHUTDOWN_GRACE and closes the pool.
pub async fn run(config: Config, shutdown: tokio_util::sync::CancellationToken) -> anyhow::Result<()>;
```
- `start_api_server(…, shutdown: CancellationToken) -> anyhow::Result<tokio::task::JoinHandle<()>>` — returns the server task instead of detaching it; graceful shutdown on `shutdown.cancelled()`.
- `run_bot_loop(…, shutdown: CancellationToken)` — every `select!` branch that used `shutdown.recv()` uses `shutdown.cancelled()`.
- `main.rs` keeps: `.env` loading, tracing setup, the `mcp` stdio path, signal handling (ctrl_c / SIGTERM → `token.cancel()`), then `app::run(config, token)`. Target: `main.rs` < 120 lines.

Rules (AGENTS.md concurrency principles):
- One root `CancellationToken`; the API server and the Matrix loop each get a `child_token()`. The per-generation `BackgroundWorkers` abort-on-drop guard stays exactly as it is (it solves reconnect duplication); it now also ends when the Matrix loop returns on cancellation.
- After cancel: `tokio::time::timeout(SHUTDOWN_GRACE, join(api_task, bot_task))`; on timeout log `warn!` naming what did not finish and continue; then `db.pool.close()` bounded by the remaining time. Total never exceeds 15 s.
- No `unwrap()`/`expect()` on these runtime paths.

- [ ] **Step 1: Failing test** — in `app.rs` tests: a unit test of the shutdown helper, e.g. `stop_within(grace, tasks)` that (a) returns promptly when tasks finish, (b) returns after ≈`grace` and reports the laggard names when a task ignores cancellation (use `tokio::time::pause()` + `advance` so the test is instant). Extract that helper so it is testable without booting the app.
- [ ] **Step 2: Implement** the helper, then `app::run`, the token plumbing, `start_api_server` returning its `JoinHandle`, and slim `main.rs`.
- [ ] **Step 3: Test the server shutdown path** — an in-process test that starts `start_api_server` on `127.0.0.1:0` (or the equivalent seam — if the function binds `API_PORT` itself, add the listener as a parameter), cancels the token, and asserts the returned `JoinHandle` completes within 1 s.
- [ ] **Step 4: Verify** — global list; confirm `grep -rn "broadcast::channel::<()>" backend/src` → none.
- [ ] **Step 5: Docs** — AGENTS.md Architecture "Single process, two front doors" and ARCHITECTURE.md: composition root is `app.rs`; shutdown is one CancellationToken tree bounded at 15 s.
- [ ] **Step 6: Commit** — `refactor: add app.rs composition root with bounded graceful shutdown`

---

## What comes next
Per-feature layering + `query!` (design step 6) — its own plan, starting with `settings` as the template feature.
