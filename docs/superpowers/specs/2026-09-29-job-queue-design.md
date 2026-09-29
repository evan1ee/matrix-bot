# Step 8: Durable Job Queue — Design

> Sub-project 1, design step 8. Binding parent: `2026-09-24-postgres-foundation-design.md` §5.1.
> Roadmap: `SPEC.md` §5. Approved by the user 2026-09-29.

## 1. Goal

Replace the in-process polling loops that deliver reminders, morning briefings and
portfolio alerts/refreshes with a durable PostgreSQL job queue: jobs survive a restart,
retries are bounded and visible, a scheduler tick run twice enqueues once, and the loops
stop being owned by a Matrix sync session.

Out of scope: the webhook delivery loop (replaced by outbox deliveries in step 9); a
Matrix sync advisory lock (deferred, R18); scheduled agent tasks (sub-project 3).

## 2. Current state

| Loop | File | Cadence | Notes |
|---|---|---|---|
| Reminders | `interface/chat/reminders/scheduler.rs` | 5 s poll of `reminders` | linear retry `30·(n+1)` s, `MAX_DELIVERY_ATTEMPTS`, mirrors `retry_count` |
| Briefing | `interface/chat/briefing/scheduler.rs` | 30 s | per-account window `[target, target+5 min)`, `last_briefing_date` guard |
| Portfolio | `interface/chat/portfolio/scheduler.rs` | alerts 2 min; price sweep + snapshot hourly | alerts DM every linked mxid |
| Webhooks | `interface/chat/webhooks.rs` | 3 s + notify | **stays until step 9** |

All four are spawned by `BotModule::spawn_background_workers` per sync generation and
owned by `BackgroundWorkers`, because they send through that generation's `Client`.
There is **no recurring-bills loop**: bills create due reminders through
`RemindersService::sync_due_reminder`, which the reminder loop delivers.

## 3. Schema — `backend/migrations/0003_jobs.sql`

Plain DDL, one transaction, LF line endings.

```sql
CREATE TABLE jobs (
    id              BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    account_id      UUID REFERENCES accounts(id) ON DELETE CASCADE,
    job_type        TEXT NOT NULL,
    payload         JSONB NOT NULL DEFAULT '{}'::jsonb,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK (status IN ('pending','running','completed','failed','cancelled')),
    priority        SMALLINT NOT NULL DEFAULT 0,
    attempts        INTEGER NOT NULL DEFAULT 0,
    max_attempts    INTEGER NOT NULL DEFAULT 3 CHECK (max_attempts >= 1),
    backoff_seconds INTEGER NOT NULL DEFAULT 10 CHECK (backoff_seconds >= 0),
    available_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    locked_at       TIMESTAMPTZ,
    locked_by       TEXT,
    started_at      TIMESTAMPTZ,
    completed_at    TIMESTAMPTZ,
    heartbeat_at    TIMESTAMPTZ,
    dedupe_key      TEXT,
    error           TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_jobs_dedupe UNIQUE (job_type, dedupe_key)
);
CREATE INDEX idx_jobs_dequeue ON jobs (priority DESC, available_at ASC) WHERE status = 'pending';
CREATE INDEX idx_jobs_running ON jobs (heartbeat_at) WHERE status = 'running';
CREATE INDEX idx_jobs_finished ON jobs (completed_at) WHERE status IN ('completed','failed','cancelled');
CREATE INDEX idx_jobs_account ON jobs (account_id);
```

- `id` is `BIGINT`, not SPEC's UUID (R17).
- `account_id` is nullable (global jobs such as `portfolio.check_alerts`). `jobs` joins
  `OWNED_TABLES`, so an account merge moves pending jobs with the account; handlers
  resolve their subject by row id, so a moved job still works.
- `NULL` dedupe keys never conflict (PostgreSQL default), so un-keyed jobs are unrestricted.

## 4. Components

| Unit | Path | Responsibility | Depends on |
|---|---|---|---|
| Queue repository | `infrastructure/postgres/repositories/jobs.rs` | Compile-checked queries only (below) | sqlx |
| Job types | `domain/jobs.rs` | `JobType` enum ↔ `&'static str`, `JobStatus`, `NewJob` builder, `backoff_delay(backoff_seconds, attempt)`, dedupe-key builders | pure |
| Runtime | `backend/src/jobs/` (`mod.rs`, `worker.rs`, `reaper.rs`, `scheduler.rs`, `registry.rs`) | `JobHandler` trait, registry, workers, heartbeat, reaper, leader-elected scheduler, shutdown | repositories, `application` services |
| Scheduling rules | `domain/<f>` | pure "what is due" functions, e.g. `briefing::due_date(settings, now) -> Option<NaiveDate>` | pure |
| Enqueue use cases | `application/<f>` | `RemindersService::enqueue_due(now)`, `BriefingService::enqueue_due(now)`, `PortfolioService::enqueue_alert_check(now)` / `enqueue_price_refresh(now)` | repositories |
| Handlers | `interface/jobs/{reminders,briefing,portfolio}.rs` | payload → service → Matrix delivery → record outcome | services, `MatrixHandle` |
| Matrix handoff | `interface/channels/matrix/handle.rs` | `MatrixHandle` = `watch::Sender<Option<Client>>`; the sync loop publishes each generation's client and clears it on exit | matrix-sdk |

### 4.1 Repository API

```rust
pub async fn enqueue(exec, job: &NewJob) -> sqlx::Result<Option<JobId>>;   // ON CONFLICT DO NOTHING; pg_notify('jobs','')
pub async fn claim(exec, worker: &str, types: &[&str]) -> sqlx::Result<Option<ClaimedJob>>;  // SPEC §5.2 CTE, SKIP LOCKED, attempts+1, heartbeat_at=NOW()
pub async fn heartbeat(exec, id, worker) -> sqlx::Result<bool>;             // only while status='running' AND locked_by=worker
pub async fn complete(exec, id, worker) -> sqlx::Result<bool>;
pub async fn retry(exec, id, worker, error: &str) -> sqlx::Result<bool>;    // attempts<max → pending, available_at += backoff; else failed
pub async fn defer(exec, id, worker, delay) -> sqlx::Result<bool>;          // pending, attempts-1, no error recorded
pub async fn fail(exec, id, worker, error: &str) -> sqlx::Result<bool>;     // permanent
pub async fn release(exec, id, worker) -> sqlx::Result<bool>;              // shutdown: pending, attempts-1
pub async fn reap_stale(exec, older_than) -> sqlx::Result<u64>;             // running & heartbeat stale → pending (or failed if attempts>=max)
pub async fn purge_finished(exec, older_than) -> sqlx::Result<u64>;
```

Every state transition after `claim` is guarded by `locked_by = worker AND status = 'running'`,
so a worker whose job was reaped cannot overwrite the new owner's state (returns `false`).
Error text stored in `error` is truncated to 2,000 chars.

### 4.2 Runtime

```rust
#[async_trait]
pub trait JobHandler: Send + Sync {
    fn job_type(&self) -> JobType;
    async fn run(&self, ctx: &JobContext, payload: serde_json::Value) -> Result<(), JobError>;
}
pub struct JobContext { pub job_id: JobId, pub attempt: i32, pub max_attempts: i32, pub cancel: CancellationToken }
pub enum JobError {
    Retryable(anyhow::Error),   // counts an attempt; backoff; failed after max_attempts
    Permanent(anyhow::Error),   // failed now
    Unavailable(Duration),      // dependency down (e.g. Matrix disconnected): defer, attempt not spent
}
```

- `JOB_WORKERS` (default 4, 1..=32, validated in `Config`) worker tasks, each looping:
  claim → run with heartbeat → record outcome; idle waits on a `PgListener` for `jobs`
  or a 5 s poll, whichever first. A listener failure degrades to polling with a
  `warn!`, never stops workers.
- Heartbeat: a sibling future renews every 30 s while `run` executes. Each handler run
  is bounded by a per-type timeout (default 5 min) → `Retryable`.
- Reaper: every 60 s, `reap_stale(2 min)`. Purge: hourly, `purge_finished(7 days)`.
- Scheduler: one task holding `pg_try_advisory_lock(SCHEDULER_LOCK)` on a dedicated
  pooled connection; retries acquisition every 30 s; while leader it ticks
  reminders (5 s), briefing (30 s), portfolio alerts (120 s), portfolio refresh (3600 s,
  first tick at start as today). Losing the connection drops leadership.
- Worker id: `<hostname>:<pid>:<uuid>`.
- Lifecycle: `app::run` spawns the runtime under a child `CancellationToken` and joins
  it in the 15 s bounded shutdown. On cancel, workers stop claiming; an in-flight
  handler gets its `ctx.cancel` and a 10 s grace, after which the job is `release`d.
- Every step is traced with `job_id`, `job_type`, `attempt`, outcome, duration.

### 4.3 Architecture rules (`tests/architecture.rs`)

- `jobs/` may not import `axum`, `matrix_sdk`, `utoipa`, `interface`, and contains no SQL.
- `interface/jobs/` is interface: no SQL.
- `domain/jobs.rs` obeys the domain crate list.

## 5. Loop migration

| Job type | Enqueued by | Dedupe key | max_attempts / backoff | Handler |
|---|---|---|---|---|
| `reminder.fire` | scheduler 5 s: `enqueue_due(now)` for every `pending` reminder with `trigger_at <= now` | `<reminder_id>:<trigger_at RFC3339>` | `MAX_DELIVERY_ATTEMPTS` / 30 s | re-load; skip unless still `pending` at that `trigger_at`; format (tz from settings); send; `record_target`; publish `ReminderTriggered`; advance (`reschedule_delivery` / `mark_delivered`). On failure mirror `retry_count = attempt` (`increment_retry`) and return `Retryable`; on the last attempt `mark_failed` |
| `briefing.send` | scheduler 30 s: accounts with briefing enabled whose local time is in the window and whose `last_briefing_date` ≠ today | `<account>:<local date>` | 3 / 60 s | generate, send to default room, `update_last_briefing_date`, publish `MorningBriefingSent` |
| `portfolio.check_alerts` | scheduler 120 s | `alerts:<unix minute bucket>` | 1 / – | current `check_price_alerts` body |
| `portfolio.refresh_prices` | scheduler 3600 s, one per holding owner | `<account>:<UTC hour>` | 3 / 60 s | `refresh_all_for_owner` + `capture_current_snapshot_for_owner` |

- Handler with no Matrix client (`MatrixHandle` empty) returns `Unavailable(30 s)`: the
  job waits without burning attempts, matching today's "loop not running while
  disconnected".
- Business rules move out of the old scheduler files: `next_occurrence` and
  `retry_delay`/format helpers into `domain/reminders`; the briefing window into
  `domain/settings` or `domain/briefing`; alert condition into `domain/portfolio/alerts`
  (already there). Formatting (`format_reminder_alert`, `format_alert_notification`)
  stays in the interface layer.
- Deleted: the three `scheduler.rs` loop functions, the three corresponding
  `spawn_background_workers` impls. `BotModule::spawn_background_workers` and
  `BackgroundWorkers` stay for the webhook worker until step 9 deletes them.
- `SystemEvent`s keep their variants and `owner_id()`; handlers publish through the
  service's `events`, exactly as today.

## 6. Error handling and idempotency

- Delivery stays at-least-once: a crash between Matrix send and commit repeats a message
  (unchanged). The dedupe key stops a double scheduler tick; the handler's re-check
  stops a cancelled/snoozed/advanced reminder from firing.
- A handler never takes an owner from the payload for authorization; the payload carries
  a row id, and the service loads the row.
- Terminal failures are logged `error!` with job id/type; the outbox alert event of
  SPEC §5.3 arrives with step 9.
- Malformed payload → `Permanent`.

## 7. Testing

- Domain: dedupe-key builders, `backoff_delay`, briefing window, `next_occurrence` (moved tests).
- Repository (`test_pool`): enqueue dedupe; claim order (priority, available_at);
  **two concurrent claimers never get the same job**; `available_at` in the future is
  not claimed; retry → backoff then `failed`; defer/release don't spend attempts;
  guarded transitions return `false` for a foreign `locked_by`; reaper; purge.
- Runtime: a fake handler per `JobError` variant; heartbeat keeps a long job from being
  reaped; cancellation releases a running job; only one of two schedulers holds the lock.
- Handlers: reminder skip-if-changed, retry mirroring, final `mark_failed`, `Unavailable`
  when no client — against a delivery seam, never a homeserver.
- Services: `enqueue_due` idempotent across two ticks.
- Existing suites, architecture test, OpenAPI zero diff, `.sqlx/` regenerated.

## 8. Rulings

- **R17:** `jobs.id` is `BIGINT` identity, not SPEC's UUID — settled "BIGINT row IDs"
  (§3 of HANDOFF). Cost if wrong: a later rename of one column type.
- **R18:** the Matrix sync advisory lock is deferred to sub-project 2's channel refactor —
  one replica exists today and the scheduler lock already prevents double delivery of
  scheduled work. Cost if wrong: a second replica would double-sync until then.
- **R19:** reminders are enqueued by a scheduler sweep, not at creation — the `reminders`
  row stays the single source of truth and no create/snooze/cancel path changes. Cost:
  up to 5 s extra latency, same as today.
- **R20:** no `bills.post_due` job — no bills loop exists; bills deliver through due
  reminders. Behaviour frozen.
- **R21:** reminder retry delay becomes exponential (30/60/120/240 s) instead of linear
  (30/60/90/120 s); `retry_count` keeps its contract meaning. Disclosed behaviour change.
- **R22:** portfolio keeps two cadences (alerts 2 min, refresh hourly) instead of
  evaluating alerts inside the refresh as design §5.1 suggested — behaviour frozen.

## 9. Acceptance

- The three loops no longer exist; their work runs as jobs; the webhook worker is unchanged.
- All gates of AGENTS.md §8 green; OpenAPI byte-identical; `.sqlx/` committed.
- `docs/architecture.md` and `docs/database.md` describe the queue; HANDOFF §4/§9 updated.
