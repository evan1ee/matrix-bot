# PostgreSQL Foundation — Design

**Date:** 2026-09-24
**Status:** Implemented. `docs/architecture.md` and `docs/database.md` describe the code as built and win wherever they differ from this design
**Parent roadmap:** `SPEC.md` (since deleted) — this document was sub-project 1 (roadmap phases 1–3)

## 1. Goal

Turn the backend into a PostgreSQL-only, layered, production-ready service without
losing a byte of the data in the live PostgreSQL deployment:

- The application database is PostgreSQL 18+ only. The dual-dialect layer
  (`db/executor.rs`, `db/dialect.rs`, `db/dialect_probe.rs`, `db/ddl.rs`,
  `db/backfill.rs`, the `db_*!`/`tx_*!` macros, `DbPool`) is deleted.
- Queries are compile-time checked (`sqlx::query!` / `query_as!`) against a committed
  offline cache.
- Schema changes are versioned, forward-only SQL migrations.
- The code is organised by layer (domain → application → interface/infrastructure),
  with the dependency rule enforced by a test.
- Background work runs on a durable PostgreSQL job queue; every state change is
  published through a transactional outbox.

Out of scope (later sub-projects, each with its own spec): Discord and Telegram
adapters, the scheduled autonomous agent runtime, object storage, frontend changes
beyond regenerated API types.

## 2. Decisions

| # | Decision | Choice | Why |
|---|---|---|---|
| D1 | Scope | Roadmap phases 1–3 | Foundation that every later phase builds on |
| D2 | Existing data | Adopt the live PostgreSQL database in place | Real data lives there; SQLite data is not carried forward |
| D3 | IDs | Row PKs stay `BIGINT`; `accounts.id` and every `owner_id` become native `UUID` | Chat commands show short numeric ids (`!todo done 3`); the account id is the one that gains from a real type and FKs |
| D4 | Query style | `query!`/`query_as!` + committed `.sqlx/` cache, `SQLX_OFFLINE=true` in builds | Type/column mismatches become compile errors — the defect class AGENTS.md's portability tables were written to catch by hand |
| D5 | sqlx version | Upgrade 0.7 → 0.9, together with matrix-sdk 0.7 → 0.18 in one step | Every query is rewritten anyway; fixes RUSTSEC-2024-0363 and has better offline tooling. Neither bump builds alone: Cargo's `links = "sqlite3"` check fires for sqlx's `libsqlite3-sys` (via its non-activated `sqlite` feature) even though it isn't enabled, and matrix-sdk 0.7's own `libsqlite3-sys` version collides with it — only matrix-sdk 0.18 resolves to a compatible version |
| D6 | Layout | Layer-first (roadmap §13) | Chosen explicitly; dependency direction is enforced by an architecture test |
| D7 | Topology | Single instance, HA-ready | SKIP LOCKED jobs, advisory-lock leader for schedulers and Matrix sync, LISTEN/NOTIFY for SSE — a second replica needs no redesign |
| D8 | Job queue | Hand-rolled on sqlx | ~300 lines, schema in our migrations, enqueue in the caller's transaction; no pre-1.0 dependency owning our schema |
| D9 | Outbox | Every event, per-destination delivery rows | One `published` flag cannot track fan-out to SSE + Matrix + webhooks; replaces `webhook_outbox` |
| D10 | Matrix crypto store | Stays matrix-sdk's own SQLite file under `data/` | matrix-sdk 0.7 has no maintained PostgreSQL store; it is SDK-owned state, not the application database |

## 3. Target Structure

```text
backend/
├── migrations/
│   ├── 0001_baseline.sql          # today's 36 tables, idempotent (IF NOT EXISTS)
│   ├── 0002_account_uuid.sql      # owner_id TEXT 'acct_<uuid>' → UUID + FKs
│   ├── 0003_jobs.sql
│   └── 0004_outbox.sql            # outbox_events + outbox_deliveries; drops webhook_outbox
├── .sqlx/                         # committed offline query cache
└── src/
    ├── main.rs                    # parse CLI (serve | mcp-stdio), call app::run
    ├── app.rs                     # composition root: AppState, task spawning, shutdown
    ├── config.rs
    ├── error.rs                   # ApiError (HTTP) + AppError (application)
    ├── domain/<feature>/          # entities, value objects, invariants, DomainError
    ├── application/<feature>/     # services: transaction + use case + outbox write
    ├── interface/
    │   ├── http/                  # Axum REST, SSE, auth extractors, OpenAPI (today's api/)
    │   ├── chat/                  # command parser, router, per-feature command handlers
    │   ├── channels/              # ChannelAdapter trait + matrix/ (today's bot/)
    │   └── mcp/
    ├── llm/                       # tool registry, bounded tool-calling loop, prompts
    ├── jobs/                      # queue, worker, reaper, scheduler, handlers/
    ├── events/                    # SystemEvent, outbox writer, dispatcher, SSE hub
    └── infrastructure/
        ├── postgres/              # pool, migrate, repositories/<feature>.rs
        └── inference/             # today's services/ (llm, stt, tts, ocr clients)
```

### 3.1 Dependency rule

```text
domain        ← depends on nothing but std, chrono, uuid, serde, thiserror
application   ← domain, infrastructure::postgres::repositories, events::outbox, jobs::queue
interface     ← application, domain (for DTO mapping), llm
llm           ← application (tools call services), infrastructure::inference
jobs/handlers ← application, infrastructure::inference, interface::channels (outbound send)
```

`tests/architecture.rs` walks `src/domain/**` and fails on any `use sqlx`, `axum`,
`matrix_sdk`, `reqwest` or `crate::{infrastructure,interface,application}`; and walks
`src/interface/**` failing on `sqlx::query`. It is a source scan, not a compiler
guarantee — cheap, and it catches the regressions that matter.

### 3.2 Repositories and services

Repositories are **concrete** modules of free functions taking
`impl PgExecutor<'_>` (pool or `&mut *tx`), returning domain types. There is no
repository trait: there is one database, and a trait with one implementation is the
speculative abstraction AGENTS.md forbids. Tests run against real PostgreSQL.

Traits exist only where a second implementation exists or is roadmapped:
`LlmClient`, `SttClient`, `TtsClient`, `OcrClient`, `ChannelAdapter`.

An application service method is the unit of transaction:

```rust
impl TodoService {
    pub async fn create(&self, owner: AccountId, cmd: CreateTodo) -> Result<Todo, AppError> {
        let todo = domain::todos::NewTodo::try_from(cmd)?;       // invariants, 500-char cap
        let mut tx = self.pool.begin().await?;
        let saved = repositories::todos::insert(&mut *tx, owner, &todo).await?;
        outbox::publish(&mut tx, owner, SystemEvent::TodoCreated { .. }).await?;
        tx.commit().await?;
        Ok(saved)
    }
}
```

Matrix command handlers, HTTP handlers, MCP tools and LLM tools all call the same
service method — the "one capability, every interface" rule from the roadmap.

### 3.3 Chat dispatch

`BotModule` is replaced by `interface/chat`: a `CommandRouter` holding
`Box<dyn CommandHandler>` per feature (prefixes `!todo`, `!expense`, …), invoked with
a platform-neutral `ChatRequest { account, channel, text, quoted, is_direct }` and
returning a `ChatReply { plain, html }`. The Matrix adapter builds `ChatRequest` from
sync events and renders `ChatReply` with `text_html`. `!link` stays handled before
sender resolution, inside the Matrix adapter, exactly as today. `WEB_ONLY` refusals
move to the router.

## 4. Data Layer

### 4.1 Pool and configuration

- `DATABASE_URL` is required and must start with `postgres://` or `postgresql://`;
  anything else refuses to boot with a message naming the variable.
- `PgPoolOptions` with `DATABASE_MAX_CONNECTIONS` and `acquire_timeout`.
  `statement_timeout` via `after_connect` is **deferred, not implemented** —
  it needs a value chosen against the longest legitimate query, including
  migrations, which is not yet known; revisit once the job queue (§6 step 8)
  gives a clearer picture of long-running statements.
- `sqlx` features: `runtime-tokio`, `tls-rustls`, `postgres`, `chrono`, `uuid`,
  `json`, `migrate`, `macros`. The `sqlite` feature is gone.

### 4.2 Migrations

- `sqlx::migrate!("./migrations")` embedded; run at boot before any task starts.
  sqlx takes its own advisory lock, so concurrent boots are safe.
- **Adoption of the live database.** `0001_baseline.sql` is the exact current schema
  (as `run_migrations` produces it on PostgreSQL today) written with
  `CREATE TABLE/INDEX IF NOT EXISTS` and `CREATE OR REPLACE` for the audit-log
  triggers. On a fresh database it builds everything; on the live one it is a no-op
  and gets recorded in `_sqlx_migrations`. Precondition, stated in the release notes:
  the live deployment runs current master once before upgrading, so its schema equals
  the baseline.
- **`0002_account_uuid.sql`** in one transaction:
  1. Guard: `DO $$ … RAISE EXCEPTION` listing any `owner_id`/`accounts.id` value not
     matching `^acct_[0-9a-f-]{36}$` (the discarded/follows sentinels in
     `ACCOUNT_ID_*` are checked explicitly).
  2. `ALTER TABLE … ALTER COLUMN owner_id TYPE UUID USING substr(owner_id, 6)::uuid`
     on `accounts.id` and every owner-keyed table.
  3. Add `REFERENCES accounts(id) ON DELETE CASCADE` on every `owner_id`.
- **Audit-log hash chain.** `compute_entry_hash` folds the owner id string into each
  digest. After 0002 the application renders `AccountId` as the canonical
  `acct_<uuid>` string when hashing, so every existing chain still verifies. A test
  pins this with a hash computed before the migration.
- **Wire format.** `AccountId(Uuid)` serialises as `acct_<uuid>` wherever an account
  id crosses the HTTP/MCP boundary today, so the frontend and MCP clients see no
  change.
- **Rehearsal.** `MIGRATE_TEST_DB` becomes `MIGRATE_TEST_DATABASE_URL`: an `--ignored`
  test runs all migrations against a restored `pg_dump` of production and asserts row
  counts per table are unchanged and every audit chain verifies. Required before
  release.
- Other column-type cleanups (`TEXT` timestamps → `TIMESTAMPTZ`, etc.) are **not** in
  0002. Each lands in its own migration when its feature is moved (§6 step 6),
  against the field that reads it — per AGENTS.md's one-at-a-time rule.

### 4.3 Tests

- The test harness stays `test_pool()` (`backend/src/db/pool.rs`) + `run_migrations_unlocked`
  (`backend/src/db/migrate.rs`): each test gets a pool against a fresh, per-test
  PostgreSQL schema with migrations applied, rather than `#[sqlx::test]`'s own
  database-per-test attribute macro.
  `DATABASE_URL`/`TEST_DATABASE_URL` must point at a server; locally
  `docker compose up -d`.
- Tests that need no database stay plain `#[test]` — domain tests in particular.
- CI: the `backend` and `backend-postgres` jobs merge into one containerised job
  with a `postgres:18` service (the supported version), running
  `cargo sqlx prepare --workspace --check`, `cargo fmt --check`, `cargo test`,
  and the OpenAPI drift check.

## 5. Jobs and Events

### 5.1 Job queue

Table per roadmap §5 plus:

- `dedupe_key TEXT` with `UNIQUE (job_type, dedupe_key)` — enqueue uses
  `ON CONFLICT DO NOTHING`, so a scheduler tick run twice enqueues once.
- `heartbeat_at TIMESTAMPTZ` — the worker renews it every 30s while a handler runs;
  the reaper reclaims `running` jobs whose heartbeat is older than 2 minutes, instead
  of a fixed 15-minute `locked_at` cutoff that would steal a long TTS job.

```rust
#[async_trait]
pub trait JobHandler: Send + Sync {
    fn job_type(&self) -> &'static str;  // registry key, e.g. "reminder.fire"
    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError>;
}
pub enum JobError { Retryable(anyhow::Error), Permanent(anyhow::Error) }
```

- `jobs::enqueue(&mut tx, NewJob { .. })` runs inside the caller's transaction.
- Workers: `JOB_WORKERS` (default 4) tasks, woken by `LISTEN jobs` or a 5s poll.
- Handlers must be idempotent; each receives `job.id` for use as an idempotency key.

Migration of today's loops (`modules/*/scheduler.rs`, `due_reminder.rs`):

| Today | After |
|---|---|
| Reminder polling loop | `reminder.fire` job enqueued at creation with `available_at = remind_at`; recurrence enqueues the next occurrence on success |
| Briefing timer | Scheduler enqueues `briefing.send` per account with `dedupe_key = <account>:<local date>` |
| Portfolio price refresh / alerts | Scheduler enqueues `portfolio.refresh_prices`; alerts evaluated in that handler |
| Recurring bills | Scheduler enqueues `bills.post_due` with a per-day dedupe key |
| Webhook delivery loop | Replaced by outbox deliveries (§5.2) |

The **scheduler** is a single loop that holds `pg_try_advisory_lock(SCHEDULER_LOCK)` on
a dedicated connection; only the holder ticks. The Matrix sync loop takes the same
kind of lock (`MATRIX_SYNC_LOCK`), so a second replica serves HTTP and runs workers
but never double-syncs.

### 5.2 Outbox

```sql
outbox_events     (id BIGINT identity, account_id UUID FK, event_type TEXT, payload JSONB,
                   created_at TIMESTAMPTZ)
outbox_deliveries (id BIGINT identity, event_id FK, destination TEXT,   -- 'webhook:<id>', 'matrix:<room>'
                   status pending|delivered|failed, attempts, available_at, last_error,
                   UNIQUE (event_id, destination))
```

- `outbox::publish(&mut tx, account, event)` inserts the event, inserts one delivery
  row per durable destination subscribed to that event type (webhooks today; chat
  notifications when an event asks for one), and calls `pg_notify('outbox', id)` —
  delivered by PostgreSQL only on commit.
- **SSE** is not a delivery row: every instance LISTENs on `outbox`, loads the event,
  and pushes it to its locally connected SSE clients whose account matches. A browser
  that misses events re-fetches on reconnect, which it already does.
- **Durable deliveries** are claimed with SKIP LOCKED, retried with exponential
  backoff, and marked `failed` after `max_attempts`. The delivery id is the
  idempotency key sent to webhook receivers (`X-Delivery-Id`).
- `SystemEvent` keeps its enum and its `owner_id()` rule; it becomes the serialised
  payload, and the process-wide `broadcast` channel is removed.
- Retention: a daily scheduler job deletes delivered events older than 30 days.

### 5.3 Operations

- `GET /health/live` → 200 always. `GET /health/ready` → 200 when the pool answers
  `SELECT 1` and (if Matrix is configured) the sync loop has completed a sync in the
  last 2 minutes; otherwise 503 with a JSON reason. `/health` is removed.
- Shutdown: one `CancellationToken` tree. On SIGINT/SIGTERM, HTTP stops accepting,
  Matrix sync stops, workers finish or release their job (status back to `pending`),
  and the pool closes — bounded at 15s.
- Logging: `tracing` JSON in production (`ENVIRONMENT=production`), spans carrying
  `account_id` and `job_id`.

## 6. Order of Work

Each step ends with master green (fmt, tests, OpenAPI drift) and deployable.

1. **Postgres-only cut** — drop the `sqlite` feature; `DbPool` wraps `PgPool` only;
   macros keep their call sites but lose the SQLite arm; SQLite `DATABASE_URL`
   refused at boot; tests run on PostgreSQL only; CI jobs merged.
2. **matrix-sdk 0.18 + sqlx 0.9, as one step** — neither half can build alone:
   Cargo's `links = "sqlite3"` check fires for sqlx's `sqlite` support crate
   (`libsqlite3-sys`) even when the `sqlite` feature itself is off, because the
   crate is still in the dependency graph, and matrix-sdk 0.7's own
   `libsqlite3-sys` version collides with it; only matrix-sdk 0.18 resolves to a
   compatible version. Fix API changes from both bumps together.
3. **Versioned migrations** — 0001 baseline; `schema*.rs`, `ddl.rs`, `backfill.rs`
   deleted.
4. **Compile-checked query pipeline** — `sqlx-cli` 0.9, `.cargo/config.toml`
   forcing `SQLX_OFFLINE=true`, first query (`health.rs`) converted, `.sqlx/`
   cache committed, CI drift check (`cargo sqlx prepare --workspace --check`).
5. **Layer skeleton** — `app.rs`, `domain/ application/ interface/ llm/ jobs/ events/
   infrastructure/` directories, architecture test, health probes, graceful shutdown;
   `api/` → `interface/http/`, `services/` → `infrastructure/inference/`,
   `bot/` → `interface/channels/matrix/`.
6. **Feature migration**, one feature per change, each moving into the layers and
   rewriting its queries with `query!`: settings, todos, reminders, finance,
   recurring bills, portfolio, bookmarks, habits, journal/life, footprints, rooms,
   AI + tools, accounts/auth/recovery, MCP, webhooks. Last change deletes
   `executor.rs`, `dialect*.rs` and the macros.
7. **Account UUID** — 0002, `owner_id` TEXT → UUID + FKs, `AccountId` type,
   rehearsal test. Placed after the per-feature `query!` rewrite (step 6) rather
   than earlier: once every query is compiler-checked, the type change is
   compiler-guided — a `cargo check` failure enumerates every call site the
   column touches, instead of relying on the portability discipline that
   compile-time checking now replaces.
8. **Job queue** — 0003, queue/worker/reaper/scheduler, then each loop moved.
9. **Outbox** — 0004, dispatcher, SSE via LISTEN, webhook deliveries; broadcaster
   and `webhook_outbox` removed.
10. **Docs** — AGENTS.md (drop the portability tables and macro rules; add layer
    rules, `cargo sqlx prepare`, migration rules) and `docs/ARCHITECTURE.md`.

## 7. Risks

| Risk | Mitigation |
|---|---|
| 0002 fails on production data | Guard clause names bad rows; rehearsal on a `pg_dump` copy is a release gate |
| Audit chains break after the UUID change | Hash input keeps `acct_<uuid>`; pinned by a test with a pre-migration digest |
| `.sqlx` cache drifts | `cargo sqlx prepare --check` in CI |
| 760 tests now need PostgreSQL locally | `docker compose up -d`; `test_pool()` + `run_migrations_unlocked` handle per-test schemas |
| Long feature-migration tail leaves two styles side by side | Macros stay working until step 6's final change deletes them; each feature change is self-contained |
| Jobs double-run on a second replica | SKIP LOCKED + dedupe keys + advisory-lock scheduler |

## 8. Acceptance

- No reference to `sqlite`, `SqlitePool`, `Dialect`, `db_*!` or `{{PK}}` in
  `backend/src` outside `interface/channels/matrix/client.rs`'s crypto-store call.
- `SQLX_OFFLINE=true cargo build` succeeds; `cargo sqlx prepare --check` clean.
- Migrations applied to a production copy: identical row counts, all audit chains
  verify.
- Killing the process mid-job: the job is re-run exactly once after restart
  (reaper or release), and a reminder is never lost or sent twice.
- A webhook receiver that is down for an hour receives every event after it
  recovers, each once per `X-Delivery-Id`.
- `tests/architecture.rs` passes.
