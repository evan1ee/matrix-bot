# Deferred Items Cleanup (Core Backend & Reliability) — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Implement task-by-task with TDD.

**Status:** in progress (2026-10-03).  
**Branch:** `master` (or feature branch if required).  
**Reference:** `docs/superpowers/HANDOFF.md` §11 (Known deferred items).

---

## Overview & Scope

This plan addresses the top 5 deferred backend bug and reliability items recorded in `HANDOFF.md` §11:

1. **AI Repository TTL Timezone Issue (`repositories/llm.rs`)**:
   - `get_or_create_conversation` calculates `cutoff` in Rust formatted as a naive string `"%Y-%m-%d %H:%M:%S"` and compares it with `updated_at` (a `TEXT` column) via `updated_at >= $3`.
   - Meanwhile, inserts and updates write `updated_at` using `CAST(CURRENT_TIMESTAMP AS TEXT)` / `NOW()::text`, which formats with local/session timezone offsets (e.g. `+08`, `+00`, `-04`).
   - If PostgreSQL runs in a timezone different from UTC or negative offsets, string comparisons fail or produce inverted TTL checks.
   - **Fix**: Use timestamp comparison `updated_at::timestamptz >= $3` binding a `DateTime<Utc>`, and persist timestamps consistently as ISO 8601 UTC strings.

2. **AI Tool `update_my_settings` Options (`llm/tools/settings.rs`)**:
   - The tool definition advertises `dev_room_id` and `max_history_turns`, but the tool execution function in `backend/src/llm/tools/settings.rs` only processes `timezone` and `currency`.
   - **Fix**: Parse and apply `dev_room_id` (handling `null`, `"none"`, or non-empty strings) and `max_history_turns` (via `domain::settings::parse_max_history_turns`).

3. **Pool Starvation Check (`config/mod.rs`)**:
   - `JOB_WORKERS` can currently be configured to equal or exceed `DATABASE_MAX_CONNECTIONS`, leaving zero or negative connections for HTTP requests, health probes, and Matrix events.
   - **Fix**: In `Config::validate`, check that `self.job_workers < self.db_max_connections as usize`, returning a descriptive error on violation.

4. **Job Notifications Silent Reconnect Wake-up (`infrastructure/postgres/job_notifications.rs`)**:
   - `JobNotifications::next` calls `self.listener.recv().await`, which swallows `Ok(None)` when sqlx 0.9 silently reconnects after an I/O drop. Any notifications sent while disconnected are lost until the worker's polling loop ticks.
   - **Fix**: Switch to `self.listener.try_recv().await?`. If it returns `None` (reconnected), return `Ok(())` so the wake notifier signals idle workers to immediately poll the queue.

5. **Bounded Reminder Enqueue Sweep (`application/reminders/delivery.rs` & `repositories/reminders.rs`)**:
   - `ReminderService::enqueue_due` calls `repositories::reminders::get_due_reminders`, which selects all pending due reminders without a `LIMIT`. Under high load, one tick could attempt to load thousands of rows into memory.
   - **Fix**: Add a `limit` parameter to `get_due_reminders` and configure a batch size in `ReminderService::enqueue_due` (e.g. `MAX_ENQUEUE_DUE_BATCH = 100`).

---

## Tasks

### Task 1: AI Repository TTL Timezone Fix (`repositories/llm.rs`)
- [x] Add unit test in `backend/src/infrastructure/postgres/repositories/llm.rs` reproducing the TTL lookup with an explicit `max_age_seconds` and verifying it matches conversations touched recently regardless of formatting.
- [x] Update `get_or_create_conversation` in `backend/src/infrastructure/postgres/repositories/llm.rs`:
  - Bind `cutoff` as `DateTime<Utc>`.
  - Use `updated_at::timestamptz >= $3` and `ORDER BY updated_at::timestamptz DESC`.
  - In `INSERT INTO ai_conversations`, write ISO 8601 UTC timestamp: `TO_CHAR(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')`.
  - In `touch_conversation`, write `TO_CHAR(NOW() AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"')`.
- [x] Regenerate/verify sqlx offline cache if needed (`cargo sqlx prepare --check`).
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib infrastructure::postgres::repositories::llm`.

### Task 2: AI Tool `update_my_settings` Options (`llm/tools/settings.rs`)
- [x] Add unit test in `backend/src/llm/tools/settings.rs` (or `backend/src/llm/tools/mod.rs`) executing `update_my_settings` with `dev_room_id` and `max_history_turns`, verifying settings are updated in database.
- [x] Update `update_my_settings` handler in `backend/src/llm/tools/settings.rs` to parse:
  - `dev_room_id`: `Some(val)` -> trimmed string, `null` / `"none"` -> `None`.
  - `max_history_turns`: `Some(val)` -> `domain::settings::parse_max_history_turns(val)?`.
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib llm::tools`.

### Task 3: Pool Starvation Validation (`config/mod.rs`)
- [x] Add unit tests in `backend/src/config/mod.rs`:
  - `validation_fails_when_job_workers_equals_or_exceeds_db_max_connections`
  - `validation_passes_when_job_workers_strictly_less_than_db_max_connections`
- [x] Update `Config::validate` in `backend/src/config/mod.rs`:
  - Reject configurations where `self.job_workers >= self.db_max_connections as usize`.
- [x] Verify: `cargo test -p matrix-bot --lib config`.

### Task 4: Job Notifications Silent Reconnect Wake-up (`job_notifications.rs`)
- [x] Add unit test in `backend/src/infrastructure/postgres/job_notifications.rs` verifying `try_recv` reconnect semantics or wake-up behavior.
- [x] Update `JobNotifications::next` in `backend/src/infrastructure/postgres/job_notifications.rs`:
  - Use `self.listener.try_recv().await?`.
  - Map `None` (silent reconnect) to `Ok(())` to wake workers.
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib infrastructure::postgres::job_notifications`.

### Task 5: Bounded Reminder Enqueue Sweep (`reminders`)
- [x] Add unit test in `repositories/reminders.rs` testing `get_due_reminders` with a `limit`.
- [x] Update `repositories::reminders::get_due_reminders` to accept `limit: i64` and add `LIMIT $3` in SQL query.
- [x] Update `ReminderService::enqueue_due` to define `const MAX_ENQUEUE_DUE_BATCH: i64 = 100;` and pass it to `get_due_reminders`.
- [x] Update callers / tests in `delivery.rs` and `reminders.rs`.
- [x] Update offline query cache `.sqlx/` if query modified.
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib application::reminders`.

### Task 6: Full Verification & Documentation Update
- [x] Run all gates from `HANDOFF.md` §5:
  - `cargo fmt --all -- --check`
  - `cargo check -p matrix-bot --all-targets`
  - `cargo test -p matrix-bot --all-targets`
  - `cargo test -p matrix-bot --test architecture`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
- [x] Update `docs/superpowers/HANDOFF.md`:
  - Mark cleared items in §11.
  - Record new rulings in §9.
