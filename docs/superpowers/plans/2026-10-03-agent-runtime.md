# Autonomous Agent Runtime & Task Framework — Implementation Plan

> **For agentic workers:** Steps use checkbox (`- [ ]`) syntax for tracking. Implement task-by-task with TDD.
> Always run the verification gates before marking a task complete.

**Status:** Planned / In Progress (2026-10-03).  
**Branch:** `feat/agent-runtime` (stacked on `master`).  
**Design Reference:** [`docs/superpowers/specs/2026-10-03-agent-design.md`](docs/superpowers/specs/2026-10-03-agent-design.md).  
**Roadmap:** `SPEC.md` Phase 5 (§1.3, §7, §8).  

---

## Overview & Scope

This plan delivers the core Autonomous Agent Runtime (`AgentService`) in `matrix-bot`, establishing an asynchronous, durable execution environment for complex multi-step tasks without degrading live conversational chat turns.

Key capabilities delivered in this milestone:
1. **Durable Job Execution:** Agent runs execute as background jobs (`agent.execute_run`) in PostgreSQL with worker lease and heartbeat safety.
2. **Persistent Audit History:** `scheduled_agent_tasks`, `agent_runs`, `agent_steps`, and `agent_approvals` track every thought, tool input/output, and token count.
3. **ReAct Reasoning Engine:** Bounded (max 15 steps) Think -> Act -> Observe loop with tool sandboxing against the Unified Tool Registry.
4. **Human-in-the-Loop Safety:** Sensitive actions (`is_sensitive == true`, e.g. trades, fund moves) halt into `waiting_approval` with an interactive chat outbox prompt (1h TTL).
5. **Interactive & Scheduled Triggers:** Custom user chat goals (`!agent run <goal>`, `/agent run <goal>`) and custom cron schedules.
6. **Extensible Plugin Framework:** `ScheduledAgentTask` trait and `ScheduledTaskRegistry` (built-in tasks deferred as [TODO] extensions).

---

## Tasks

### Task 1: Migration `0010_agent_runtime.sql` & Domain Types (`domain/agent/`)
- [x] Create migration `backend/migrations/0010_agent_runtime.sql`:
  - `scheduled_agent_tasks` (id, account_id, task_id, cron_expression, enabled, config, last_run_at, next_run_at, created_at, updated_at).
  - `agent_runs` (id, scheduled_task_id, account_id, task_name, goal, origin_conversation, status, max_steps, current_step, total_tokens, error, started_at, completed_at, updated_at).
  - `agent_steps` (id, run_id, step_number, thought, tool_name, tool_input, tool_output, status, tokens_used, created_at).
  - `agent_approvals` (id, run_id, account_id, tool_name, tool_input, status, expires_at, resolved_at, created_at).
  - Relevant indexes for active runs, due tasks, pending approvals, and steps.
- [x] Register new tables in `backend/src/domain/accounts/mod.rs` under `ACCOUNT_ID_FOLLOWS`:
  - `"scheduled_agent_tasks"`, `"agent_runs"`, `"agent_approvals"`.
- [x] Implement pure domain types in `backend/src/domain/agent/mod.rs`:
  - Newtypes: `AgentRunId`, `AgentStepId`, `ApprovalId`, `TaskId`.
  - Enums: `RunStatus` (`Planning`, `Executing`, `WaitingApproval`, `Completed`, `Failed`, `Cancelled`), `StepStatus` (`Success`, `Failed`, `Blocked`), `ApprovalStatus` (`Pending`, `Approved`, `Rejected`, `Expired`).
  - Entities: `AgentRun`, `AgentStep`, `ApprovalRequest`, `ScheduledTaskDefinition`.
  - Error enum: `AgentError`.
  - Invariants: step monotonicity, max step clamp (1..=25, default 15), goal length cap (1000 chars), valid status state transitions, approval expiry logic.
- [x] Add unit tests in `domain/agent/mod.rs` asserting all invariants and state transitions.
- [x] Verify: `cargo test -p matrix-bot --lib domain::agent`.

### Task 2: Repository Persistence (`infrastructure/postgres/repositories/agent.rs`)
- [x] Create `backend/src/infrastructure/postgres/repositories/agent.rs`:
  - Implement `create_run`, `find_run_by_id`, `find_active_run_for_account`, `update_run_status`.
  - Implement `record_step`, `get_run_steps`.
  - Implement `create_approval`, `find_pending_approval`, `resolve_approval`.
  - Implement `list_due_scheduled_tasks` (`FOR UPDATE SKIP LOCKED`), `update_task_next_run`.
- [x] Add repository tests in `infrastructure/postgres/repositories/agent.rs`:
  - Round-trip creation and retrieval of runs, steps, and approvals.
  - Multi-tenant owner isolation test (`assert_owner_isolation`).
  - Due task lock and concurrency test.
- [x] Refresh `.sqlx/` query cache with `cargo sqlx prepare`.
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib infrastructure::postgres::repositories::agent`.

### Task 3: Application Service & Domain Events (`application/agent/`)
- [x] Define agent events in `backend/src/domain/events.rs`:
  - `AgentRunStarted { run_id, account_id, goal }`
  - `AgentStepCompleted { run_id, account_id, step_number, tool_name }`
  - `AgentApprovalRequired { run_id, account_id, approval_id, tool_name }`
  - `AgentRunCompleted { run_id, account_id, status }`
  - Ensure every event implements `owner_id() -> AccountId`.
- [x] Implement `AgentService` in `backend/src/application/agent/mod.rs`:
  - `start_user_goal(account_id, origin_conversation, goal) -> Result<AgentRun, AppError>` (enforces max 1 active run).
  - `get_active_run(account_id) -> Result<Option<AgentRun>, AppError>`
  - `get_run(account_id, run_id) -> Result<AgentRun, AppError>`
  - `get_run_steps(account_id, run_id) -> Result<Vec<AgentStep>, AppError>`
  - `approve_action(account_id, approval_id) -> Result<(), AppError>`
  - `reject_action(account_id, approval_id, reason) -> Result<(), AppError>`
  - `cancel_run(account_id, run_id) -> Result<(), AppError>`
  - `enqueue_due_scheduled_tasks() -> Result<usize, AppError>`
- [x] Add application service unit/integration tests:
  - Single active run constraint check (second concurrent goal fails with Conflict).
  - Transaction atomicity & outbox event writing.
- [x] Wire `AgentService` into `AppServices` in `backend/src/application/mod.rs`.
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib application::agent`.

### Task 4: ReAct Execution Engine & Tool Sandboxing (`llm/agent/`)
- [x] Create `backend/src/llm/agent/` module:
  - `prompts.rs`: System prompt for autonomous agent scratchpad (`Thought:`, `Action:`, `Action Input:`, `Observation:`, `Final Answer:`).
  - `sandbox.rs`: Tool filtering against `task.allowed_tools()` whitelist; detection of `is_sensitive == true`.
  - `engine.rs`: Single step execution (`evaluate_step`):
    - Constructs conversation history from goal + recorded `agent_steps`.
    - Invokes `LlmClient::chat_completion`.
    - Parses thought and tool call.
    - If tool is sensitive, returns `StepOutcome::ApprovalRequired`.
    - If safe tool, executes against `ToolRegistry` and returns `StepOutcome::Executed`.
    - If final answer reached, returns `StepOutcome::Completed`.
- [x] Add unit tests in `llm/agent/` verifying prompt formatting, tool filtering, sensitive interception, and thought parsing.
- [x] Verify: `cargo test -p matrix-bot --lib llm::agent`.

### Task 5: Durable Job Runtime Handler (`interface/jobs/agent.rs`)
- [x] Create `backend/src/interface/jobs/agent.rs`:
  - Register job type `agent.execute_run { run_id: Uuid, origin_conversation: Option<String> }`.
  - Implement worker loop with heartbeat:
    - Loads run and validates state.
    - Advances ReAct step via `llm::agent::engine`.
    - If `ApprovalRequired`, records approval, marks run `waiting_approval`, and publishes outbox alert.
    - If `Executed`, persists step and continues loop while time/lease permits.
    - If `Completed`, synthesizes summary, posts final brief to `origin_conversation` via outbox, and marks run `completed`.
- [x] Register job handler in `interface/jobs/mod.rs` and scheduler sweep for due scheduled tasks.
- [x] Add job handler tests with mock LLM responses.
- [x] Verify: `$env:TEST_DATABASE_URL="postgres://postgres:bot@localhost:55433/postgres"; cargo test -p matrix-bot --lib interface::jobs::agent`.

### Task 6: Chat & HTTP Interfaces
- [x] Implement `AgentModule` in `backend/src/interface/chat/agent.rs`:
  - `!agent run <goal>`: Starts goal, acknowledges immediately with run ID.
  - `!agent status [run_id]`: Prints progress and step count.
  - `!agent confirm <approval_id>`: Confirms sensitive action.
  - `!agent reject <approval_id>`: Rejects sensitive action.
  - `!agent cancel [run_id]`: Cancels active run.
  - `!agent list`: Lists recent runs and scheduled tasks.
- [x] Register `AgentModule` in `interface/channels/matrix/modules.rs`.
- [x] Implement REST endpoints in `backend/src/interface/http/agent.rs`:
  - `POST /api/v1/agent/runs`
  - `GET /api/v1/agent/runs`
  - `GET /api/v1/agent/runs/{id}`
  - `POST /api/v1/agent/runs/{id}/cancel`
  - `POST /api/v1/agent/approvals/{id}/approve`
  - `POST /api/v1/agent/approvals/{id}/reject`
  - `GET /api/v1/agent/tasks`
  - `PUT /api/v1/agent/tasks/{id}`
- [x] Register routes in `interface/http/routes.rs`.
- [x] Regenerate OpenAPI spec: `cargo run --bin generate-openapi`.
- [x] Verify: `cargo test -p matrix-bot --lib interface::chat::agent` & `interface::http::agent`.

### Task 7: Extensible Plugin Foundation (`ScheduledAgentTask` & Registry)
- [x] Implement `ScheduledAgentTask` trait and `ScheduledTaskRegistry` in `backend/src/domain/agent/task.rs`.
- [x] Stub the planned built-in tasks (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, etc.) as documented TODO extensions.
- [x] Wire registry into `AppServices`.
- [x] Verify: `cargo test -p matrix-bot --lib domain::agent::task`.

### Task 8: Full Verification & Documentation
- [x] Run all verification gates:
  - `cargo fmt --all -- --check`
  - `cargo check -p matrix-bot --all-targets`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test -p matrix-bot --all-targets`
  - `cargo test -p matrix-bot --test architecture`
  - `cargo sqlx prepare --workspace --check -- -p matrix-bot --all-targets`
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
- [x] Update `docs/superpowers/HANDOFF.md` (§9 Rulings, §11 Deferred items).
- [x] Commit with conventional commits.

