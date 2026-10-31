# Agent Task Controls Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user pause, resume, run-now and delete a scheduled agent task from REST, chat and the web UI.

**Architecture:** Repository gets two owner-scoped functions; `AgentService` gets `set_task_enabled`, `trigger_task_now`, `delete_task` (transactions + outbox); HTTP and `!agent task …` call the service; the tab's dead `handleToggle` is replaced by real controls.

**Tech Stack:** Rust/Axum/SQLx, utoipa OpenAPI, React 19 + React Query.

**Spec:** `docs/superpowers/specs/2026-10-31-feature-blueprints.md` — Blueprint 10.

## Global Constraints

- Resume recomputes `next_run_at` strictly after `now` (`CronSchedule::next_after(now)`); pause leaves it untouched.
- At most one active run per account: trigger returns `AppError::Conflict` (HTTP 409) when `find_active_run_for_account` finds one.
- Every mutation publishes a `SystemEvent` with an owner in the same transaction.
- Rulings: (1) trigger reuses `SystemEvent::AgentRunStarted` instead of a new `AgentRunCreated` variant (same meaning, no duplicate); (2) new variants `AgentTaskUpdated { owner_id, task_id, enabled }` and `AgentTaskDeleted { owner_id, task_id }`; (3) trigger does not move `next_run_at` and works on paused tasks; (4) `set_scheduled_task_state` returns the updated row (`Option<ScheduledTaskDefinition>`) rather than `bool` so the service/API can answer without a second read; (5) deletion leaves past runs (FK is `ON DELETE SET NULL`).
- Task ids are validated by `TaskId::new` at the boundary; unknown id → 404.

## Review Focus

- Another account's task id → 404 and the row is unchanged (pause, resume, trigger, delete).
- Resume of a task whose `next_run_at` is days in the past schedules in the future, not now.
- Trigger while a run is active → 409, no run row, no job.
- Trigger on a task id with no plugin in the registry still creates a run (falls back to the generic goal, same as the scheduler).
- Paused task is not picked up by `enqueue_due_scheduled_tasks`.

## Task 1: Repository functions

**Files:** Modify `backend/src/infrastructure/postgres/repositories/agent/tasks.rs` (+ tests in the module that already tests agent repos)

**Interfaces:**
- Produces: `set_scheduled_task_state(conn: impl PgExecutor<'e>, account_id: &AccountId, task_id: &TaskId, enabled: bool, next_run_at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> Result<Option<ScheduledTaskDefinition>, sqlx::Error>` (`next_run_at` `None` keeps the stored value); `delete_scheduled_task(conn, &AccountId, &TaskId) -> Result<bool, sqlx::Error>`.

- [ ] Failing tests: `set_state_is_owner_scoped_and_keeps_next_run_when_none`, `delete_is_owner_scoped`.
- [ ] Implement with `UPDATE … SET enabled=$3, next_run_at=COALESCE($4,next_run_at), updated_at=$5 WHERE account_id=$1 AND task_id=$2 RETURNING <ScheduledTaskRow columns>` and `DELETE … WHERE account_id=$1 AND task_id=$2`. Re-export if `agent/mod.rs` uses explicit exports.
- [ ] Regenerate `.sqlx/`; tests pass; commit `feat: repository ops to toggle and delete scheduled agent tasks`.

## Task 2: Service + events

**Files:** Modify `backend/src/domain/events.rs` (variants, `owner_id()`, type name), `backend/src/application/agent/tasks.rs`; test alongside.

**Interfaces:**
- Produces on `AgentService`: `set_task_enabled(&self, &AccountId, &TaskId, bool) -> Result<ScheduledTaskDefinition, AppError>`; `trigger_task_now(&self, &AccountId, &TaskId) -> Result<AgentRun, AppError>`; `delete_task(&self, &AccountId, &TaskId) -> Result<(), AppError>`.
- Refactor: extract the run-creation block of `enqueue_due_scheduled_tasks` into a private `create_scheduled_run(&self, tx, &ScheduledTaskDefinition, now) -> Result<AgentRun, AppError>` (creates run, enqueues `agent_execute`; the manual trigger then publishes `AgentRunStarted`); the scheduler keeps its behaviour exactly (it publishes no event today; only manual trigger does).

- [ ] Failing tests: resume recomputes `next_run_at > now` from a stale value; pause keeps it and the scheduler skips it; trigger creates one run + one `agent.execute_run` job + `AgentRunStarted`; trigger with an active run → `Conflict` and zero new rows; stranger → `NotFound` for all three; delete removes the row, outbox has `AgentTaskDeleted`; outbox for pause has `AgentTaskUpdated`.
- [ ] Implement; `cargo test -p matrix-bot agent`; commit `feat: pause, resume, trigger and delete scheduled agent tasks`.

## Task 3: REST + OpenAPI

**Files:** Modify `backend/src/interface/http/agent.rs`, `backend/src/interface/http/routes.rs`, `backend/openapi.json`, `frontend` generated types (`npm --prefix frontend run generate-api`).

**Interfaces:** `PATCH /agent/tasks/:id/state` body `SetTaskStateRequest { enabled: bool }` → `ScheduledTaskResponse`; `POST /agent/tasks/:id/trigger` → `AgentRunResponse` (200, like `POST /agent/runs`); `DELETE /agent/tasks/:id` → 204. Unknown/foreign id → 404, active run → 409.

- [ ] Failing HTTP tests in the existing agent http test module: round trip pause→resume, trigger 200 then second trigger 409, delete 204 then 404, unauthenticated 401.
- [ ] Implement handlers (map invalid id to `AppError::Validation`), register routes (`.route("/agent/tasks/:id", put(..).delete(..))`), regenerate OpenAPI (`cargo run --bin generate-openapi`) and frontend types.
- [ ] Commit `feat: agent task control endpoints`.

## Task 4: Chat commands

**Files:** Modify `backend/src/interface/chat/agent.rs` (+ its test).

- [ ] Failing test: `!agent task pause|resume|run|delete <id>` replies, `!agent task` lists usage, bad id replies with an error via `error_reply`.
- [ ] Add the four subcommands to `commands()` help and `handle_command`, delegating to the service; escape task ids with `html_escape`.
- [ ] Commit `feat: !agent task chat commands`.

## Task 5: Web UI

**Files:** Modify `frontend/src/modules/agent/api.ts` (`useSetTaskEnabled`, `useTriggerTask`, `useDeleteTask`), `ScheduledTasksTab.tsx` (remove dead `handleToggle`), `frontend/e2e/fake-api.ts`, new `frontend/e2e/agent-tasks.spec.ts`.

- [ ] Failing Playwright spec: toggle pause → badge shows "Paused"; "Run now" → success toast and run appears; delete asks confirmation then removes the card.
- [ ] Implement buttons with accessible labels, `transition-colors duration-150`, per Design.md; invalidate `scheduled-tasks` and runs queries; refresh also on `AgentTaskUpdated`/`AgentTaskDeleted` SSE types in `useSystemEvents.ts`.
- [ ] `npm --prefix frontend run build` and `test:e2e` pass; commit `feat: scheduled task controls in the agent console`.

## Task 6: Gates and handoff

- [ ] fmt, clippy `-D warnings`, full `cargo test -p matrix-bot --all-targets --no-fail-fast -- --test-threads=2`, `cargo sqlx prepare --check`, openapi drift check; update `docs/superpowers/HANDOFF.md` (status + rulings above) and `docs/api.md` endpoint list.
