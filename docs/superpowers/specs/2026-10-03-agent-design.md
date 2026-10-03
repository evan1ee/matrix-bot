# Sub-project 4 — Autonomous Agent Runtime & Task Framework Design

**Status:** Draft for review (2026-10-03).  
**Roadmap:** `SPEC.md` Phase 5, §1.3 (Interactive User Input vs Scheduled Autonomous Agent) & §8 (Scheduled Autonomous Agent Runtime & Extensible Task Framework).  
**Target Branch:** `feat/agent-runtime` (stacked on `master`).  

---

## 1. Executive Summary & Vision

This design establishes a durable, safe, and extensible **Autonomous Agent Runtime** (`AgentService`) for `matrix-bot`. It provides a unified orchestration engine for:
1. **Scheduled Background Workflows:** Periodic autonomous runs triggered by cron or database timers (e.g., Sunday weekly digest briefings, monthly financial audits, portfolio rebalance alerts, and automated database/storage maintenance).
2. **User-Triggered Asynchronous Goals:** Long-running, multi-step goals initiated on-demand by users via chat commands (`!agent run <goal>`, `/agent run <goal>`) or the Web UI (`POST /api/v1/agent/runs`).

The agent runtime is decoupled from real-time interactive chat turns. Routine queries and direct single-action requests remain in the fast-path (deterministic CLI or bounded single-turn tool calling, ≤ 5 rounds, 0 background state). Complex, multi-step autonomous tasks run in the background orchestrated by the **PostgreSQL durable job queue** (`jobs` table, `SKIP LOCKED` leases, heartbeat reapers), recording every thought, action, tool input, output, and token expenditure to `agent_runs` and `agent_steps`.

```text
               ┌────────────────────────────────────────────────────────┐
               │                    TRIGGER SOURCES                     │
               │  1. PostgreSQL Cron Schedule (Due Task Sweep)          │
               │  2. User Chat / Slash Command (!agent run <goal>)      │
               │  3. Web Console API Request (POST /api/v1/agent/runs)  │
               └───────────────────────────┬────────────────────────────┘
                                           │
                                           ▼
               ┌────────────────────────────────────────────────────────┐
               │            AgentService::enqueue_run(...)              │
               │  • Validates account limits (max 1 active run)         │
               │  • Creates `agent_runs` record (status: 'planning')    │
               │  • Enqueues durable job: `agent.execute_run`           │
               │  • Publishes SystemEvent::AgentRunStarted              │
               └───────────────────────────┬────────────────────────────┘
                                           │
                                           ▼
               ┌────────────────────────────────────────────────────────┐
               │            DURABLE JOB WORKER (SKIP LOCKED)            │
               │  `interface/jobs/agent.rs` handles `agent.execute_run` │
               │  • Heartbeat protects long-running execution           │
               │  • Loads run state & task plugin sandbox               │
               └───────────────────────────┬────────────────────────────┘
                                           │
                                           ▼
               ┌────────────────────────────────────────────────────────┐
               │              ReAct EXECUTION ENGINE (Loop)             │
               │  For step = 1..=max_steps (capped at 15):              │
               │  1. THINK: LLM analyzes goal + step history            │
               │  2. SENSITIVITY CHECK:                                 │
               │     ├── If tool is sensitive (is_sensitive=true):      │
               │     │   ► Create `agent_approvals` entry               │
               │     │   ► Set run status 'waiting_approval'            │
               │     │   ► Post Outbox confirmation to chat             │
               │     │   ► HALT & YIELD until user confirms             │
               │     └── If safe tool:                                  │
               │         ► ACT: Execute against Unified Tool Registry   │
               │         ► OBSERVE: Capture output & token metrics      │
               │         ► INSERT INTO `agent_steps`                    │
               │         ► Optional stream/delta to Origin Channel      │
               │  3. SYNTHESIZE & COMPLETE:                             │
               │     ► Formulate final brief & artifacts (TTS audio)    │
               │     ► Outbox delivery to user's linked chat channels   │
               │     ► Mark run 'completed'                             │
               └────────────────────────────────────────────────────────┘
```

---

## 2. Settled Decisions (from User Interview & Architecture Rules)

| # | Decision | Rationale | Cost if Wrong |
|---|---|---|---|
| **D1** | **Unified Engine:** Single runtime serves both scheduled cron tasks and ad-hoc chat goals (`!agent run <goal>`). | Avoids duplicate agent engines; all multi-step reasoning shares one state machine, step log, and sandbox. | Slight extra generality in task goal synthesis. |
| **D2** | **Durable Job Queue Execution:** Agent steps run inside `interface/jobs/agent.rs` via `jobs` table (`agent.execute_run`), not detached Tokio tasks. | Crash recovery, worker lease timeouts, heartbeat protection, graceful shutdown joining, and zero stranded memory state. | Job handler payload serialization overhead. |
| **D3** | **Persistent Audit State:** Three dedicated PostgreSQL tables: `scheduled_agent_tasks`, `agent_runs`, `agent_steps`, and `agent_approvals`. | Full observability of LLM thoughts, tool calls, results, and token costs; users and web dashboards can inspect live traces. | Storage for step logs (purged after retention period). |
| **D4** | **Least-Privilege Tool Sandboxing:** Each task declares an allowed tool whitelist (`allowed_tools()`). Ad-hoc goals inherit a safe default whitelist. | Prevents model hallucinations from invoking arbitrary dangerous tools in background workflows. | Tasks must explicitly register needed capabilities. |
| **D5** | **Interactive Human-in-the-Loop Approvals:** If an agent selects a tool marked `is_sensitive = true` (e.g. `portfolio_execute_trade`, fund transfers), the run transitions to `waiting_approval` and sends an approval request via chat outbox with 1-hour TTL. | Complete safety guarantee: no destructive autonomous action occurs without explicit human authorization. | Run pauses until human confirms via `!agent confirm <id>` or web. |
| **D6** | **Omnichannel Outbox Notifications:** Intermediate major progress markers and the final synthesized brief are dispatched through the Transactional Outbox to the origin chat channel (Matrix, Discord, Telegram, Email). | User stays informed in their everyday chat client without needing to poll a web browser dashboard. | A few outbox delivery records per run. |
| **D7** | **Strict Concurrency & Step Caps:** Maximum **1 active run per account**; maximum **15 steps per run**; maximum **5-minute execution budget** per run. | Prevents runaway token consumption, infinite loops, and connection pool starvation. | Tasks exceeding 15 steps fail gracefully with a partial summary. |
| **D8** | **Pluggable Task Trait:** Built-in scheduled tasks implement `ScheduledAgentTask` trait (`task_id`, `default_cron`, `allowed_tools`, `build_goal`, `on_completed`). | Clean open-closed extensibility: adding new scheduled agents requires zero changes to core scheduler or runner loops. | None; trait boundary is justified by multiple distinct plugins. |
| **D9** | **Account Ownership Isolation:** Every agent table carries `account_id UUID REFERENCES accounts(id)` and is registered in `ACCOUNT_ID_FOLLOWS` in `domain/accounts/mod.rs`. | Strict tenant isolation and clean merge semantics during account linking. | None; mandatory repository invariant. |
| **D10** | **Direct Services Delegation:** Agent tool execution routes strictly through `AppServices` into domain services, never bypassing transaction boundaries or directly querying SQL. | Enforces all domain invariants, outbox events, and audit logging uniformly across CLI, REST, MCP, and Agent callers. | None; required by `AGENTS.md` Rule 2 & 3. |

---

## 3. Architecture & Layers

Following the project's layer-first layout under `backend/src/`:

```text
domain/agent/                   Entities, state machines, invariants, domain errors (pure, synchronous)
application/agent/              AgentService: use cases, run lifecycle, transactions, outbox events
infrastructure/postgres/repositories/agent.rs  PostgreSQL persistence (compile-checked query!)
llm/agent/                      ReAct loop runner, prompt templates, tool sandboxing, token metrics
interface/jobs/agent.rs         Durable job worker handler for `agent.execute_run`
interface/chat/agent.rs         BotModule: `!agent run`, `!agent confirm`, `!agent cancel`, `!agent list`
interface/http/agent.rs         Axum REST handlers + DTOs (/api/v1/agent/*)
```

### 3.1 Domain Layer: `domain/agent/` (Pure)
- **Zero I/O, tokio, or SQL dependencies.**
- Value objects and models:
  - `TaskId`: Validated identifier for built-in or custom tasks (`^[a-z0-9_-]{3,64}$`).
  - `AgentRunId`: Strong newtype wrapping `Uuid`.
  - `AgentStepId`: Strong newtype wrapping `Uuid`.
  - `ApprovalId`: Strong newtype wrapping `Uuid`.
  - `RunStatus`: Enum: `Planning`, `Executing`, `WaitingApproval`, `Completed`, `Failed`, `Cancelled`.
  - `StepStatus`: Enum: `Success`, `Failed`, `Blocked`.
  - `ApprovalStatus`: Enum: `Pending`, `Approved`, `Rejected`, `Expired`.
  - `AgentRun`: Entity tracking run goal, current step count, max steps, token usage, timestamps, and error messages.
  - `AgentStep`: Entity tracking step number, model thought, tool name, tool input JSON, tool output JSON, status, and tokens used.
  - `ApprovalRequest`: Entity tracking sensitive tool call, arguments, verification token, expiry (1 hour TTL), and status.
  - `ScheduledTaskDefinition`: Entity tracking cron schedule, enabled state, custom parameters, and next trigger instant.
- Pure invariants:
  - Max step ceiling cannot exceed 25 (default 15).
  - Goal length capped at 1000 characters.
  - Step number must be strictly monotonic (1, 2, ...).
  - Cannot transition from terminal states (`Completed`, `Failed`, `Cancelled`).
  - Approval expiry computed as `created_at + 1 hour`.

### 3.2 Infrastructure Layer: `infrastructure/postgres/repositories/agent.rs`
- Implements compile-checked queries using `sqlx::query!` / `query_as!`.
- Core repository operations:
  - `find_active_run_for_account(executor, account_id) -> Option<AgentRunRow>`: Enforces 1 active run limit.
  - `create_run(executor, run) -> AgentRunRow`
  - `update_run_status(executor, run_id, status, error, completed_at)`
  - `record_step(executor, step) -> AgentStepRow`
  - `get_run_steps(executor, run_id) -> Vec<AgentStepRow>`
  - `create_approval(executor, approval) -> ApprovalRow`
  - `find_pending_approval(executor, approval_id) -> Option<ApprovalRow>`
  - `resolve_approval(executor, approval_id, status, resolved_at)`
  - `list_due_scheduled_tasks(executor, now, limit) -> Vec<ScheduledTaskRow>`: Uses `FOR UPDATE SKIP LOCKED`.
  - `update_task_next_run(executor, task_id, last_run, next_run)`

### 3.3 LLM & Execution Layer: `llm/agent/`
- Implements the autonomous ReAct (Reasoning + Acting + Observing) iteration engine:
  ```rust
  pub struct ReActEngine {
      llm_client: Arc<LlmClient>,
      tool_registry: Arc<ToolRegistry>,
  }
  ```
- **Prompt Engineering (`llm/agent/prompts.rs`):**
  - System prompt establishes identity, available sandboxed tools, scratchpad format (`Thought:`, `Action:`, `Action Input:`, `Observation:`, `Final Answer:`), and safety rules.
  - Context includes user profile preferences, current UTC timestamp, and chronological list of prior steps in the run.
- **Sandboxing:**
  - Filters schemas passed to LLM to strictly match `task.allowed_tools()`.
  - Intercepts selected tool before invocation: if `tool.is_sensitive`, emits `StepResult::NeedsApproval { tool_name, arguments }`.
- **Token Accounting:**
  - Extracts prompt and completion token counts from provider responses, accumulating into `agent_runs.total_tokens` and per-step records.

### 3.4 Application Layer: `application/agent/`
- `AgentService { pool, job_service, outbox_service, task_registry, llm_engine }`:
  - `start_user_goal(account_id, origin_conversation, goal) -> Result<AgentRun, AppError>`:
    - Verifies no active run exists for account (Conflict if running).
    - Inserts `agent_runs` record in transaction.
    - Enqueues durable job `agent.execute_run { run_id, origin_conversation }`.
    - Publishes `SystemEvent::AgentRunStarted`.
  - `advance_run(run_id) -> Result<RunOutcome, AppError>`:
    - Called by job worker. Loads run and steps.
    - Evaluates next ReAct step.
    - If step requires approval: stores `agent_approvals`, marks run `waiting_approval`, publishes `SystemEvent::AgentApprovalRequired`, posts notification via outbox.
    - If step is regular tool: executes against `services`, stores `agent_steps`, publishes `SystemEvent::AgentStepCompleted`.
    - If goal achieved or step limit reached: synthesizes final brief, publishes `SystemEvent::AgentRunCompleted`, delivers final brief to origin chat channel.
  - `approve_action(account_id, approval_id) -> Result<(), AppError>`:
    - Verifies ownership and `Pending` status.
    - Marks approval `Approved`, transitions run back to `Executing`.
    - Re-enqueues `agent.execute_run` job to continue.
  - `reject_action(account_id, approval_id, reason) -> Result<(), AppError>`:
    - Marks approval `Rejected`.
    - Injects rejection observation into agent steps so model can plan an alternative path or complete gracefully.
  - `cancel_run(account_id, run_id) -> Result<(), AppError>`:
    - Transitions run to `Cancelled`.
  - `enqueue_due_scheduled_tasks() -> Result<usize, AppError>`:
    - Periodic scheduler sweep (every 60s) claiming due `scheduled_agent_tasks`.
    - Instantiates run from `ScheduledAgentTask::build_goal(&ctx)` and enqueues execution job.

### 3.5 Job Runtime: `interface/jobs/agent.rs`
- Implements `JobHandler` for job type `agent.execute_run`:
  ```rust
  pub struct AgentExecuteHandler {
      services: Arc<AppServices>,
  }
  ```
- Steps:
  1. Claims job with worker lease and starts heartbeat.
  2. Invokes `AgentService::advance_run(payload.run_id)`.
  3. If run is still `Executing` (more steps needed), reenqueues next step evaluation or loops within time budget (up to 60s per job attempt).
  4. If run enters `WaitingApproval`, job finishes successfully (yielding CPU/worker until user interaction).
  5. If run completes or fails, records completion metrics and finishes job.

### 3.6 Interface Layer (Chat & HTTP)
- **Chat (`interface/chat/agent.rs` — `AgentModule`):**
  - `!agent run <goal>` / `/agent run <goal>`: Starts an asynchronous goal. Replies immediately with run ID and acknowledgment.
  - `!agent status [run_id]`: Displays current step, thought summary, and status.
  - `!agent confirm <approval_id>`: Confirms a blocked sensitive action.
  - `!agent reject <approval_id>`: Rejects a blocked sensitive action.
  - `!agent cancel [run_id]`: Aborts the active run.
  - `!agent list`: Lists recent agent runs and schedules.
- **HTTP REST API (`interface/http/agent.rs`):**
  - `POST /api/v1/agent/runs`: Start ad-hoc run.
  - `GET /api/v1/agent/runs`: List recent runs for authenticated account.
  - `GET /api/v1/agent/runs/{id}`: Detailed run status with all steps and thoughts.
  - `POST /api/v1/agent/runs/{id}/cancel`: Cancel active run.
  - `POST /api/v1/agent/approvals/{id}/approve`: Approve pending sensitive action.
  - `POST /api/v1/agent/approvals/{id}/reject`: Reject pending sensitive action.
  - `GET /api/v1/agent/tasks`: List scheduled tasks.
  - `PUT /api/v1/agent/tasks/{id}`: Update task cron schedule or enabled state.

---

## 4. Database Schema & Migration

### Migration `0010_agent_runtime.sql`

```sql
-- 1. Scheduled Agent Task Configurations
CREATE TABLE scheduled_agent_tasks (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    task_id         TEXT NOT NULL,
    cron_expression TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT TRUE,
    config          JSONB NOT NULL DEFAULT '{}'::jsonb,
    last_run_at     TIMESTAMPTZ,
    next_run_at     TIMESTAMPTZ NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_account_task UNIQUE (account_id, task_id)
);

CREATE INDEX idx_scheduled_agent_tasks_due 
    ON scheduled_agent_tasks(next_run_at) 
    WHERE enabled = TRUE;

-- 2. Agent Execution Runs
CREATE TABLE agent_runs (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    scheduled_task_id   UUID REFERENCES scheduled_agent_tasks(id) ON DELETE SET NULL,
    account_id          UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    task_name           TEXT NOT NULL,
    goal                TEXT NOT NULL,
    origin_conversation TEXT, -- e.g. "matrix:!room:server" or "telegram:12345"
    status              TEXT NOT NULL DEFAULT 'planning'
                        CHECK (status IN ('planning', 'executing', 'waiting_approval', 'completed', 'failed', 'cancelled')),
    max_steps           INTEGER NOT NULL DEFAULT 15,
    current_step        INTEGER NOT NULL DEFAULT 0,
    total_tokens        INTEGER NOT NULL DEFAULT 0,
    error               TEXT,
    started_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    completed_at        TIMESTAMPTZ,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_runs_account ON agent_runs(account_id, started_at DESC);
CREATE INDEX idx_agent_runs_active ON agent_runs(account_id) WHERE status IN ('planning', 'executing', 'waiting_approval');

-- 3. Granular Step Execution History
CREATE TABLE agent_steps (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    step_number     INTEGER NOT NULL,
    thought         TEXT,
    tool_name       TEXT,
    tool_input      JSONB,
    tool_output     JSONB,
    status          TEXT NOT NULL CHECK (status IN ('success', 'failed', 'blocked')),
    tokens_used     INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT uq_run_step UNIQUE (run_id, step_number)
);

CREATE INDEX idx_agent_steps_run ON agent_steps(run_id, step_number ASC);

-- 4. Human-in-the-Loop Approvals
CREATE TABLE agent_approvals (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id          UUID NOT NULL REFERENCES agent_runs(id) ON DELETE CASCADE,
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    tool_name       TEXT NOT NULL,
    tool_input      JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                        CHECK (status IN ('pending', 'approved', 'rejected', 'expired')),
    expires_at      TIMESTAMPTZ NOT NULL,
    resolved_at     TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_approvals_pending ON agent_approvals(account_id, status) WHERE status = 'pending';
```

### Table Ownership Classification (`domain/accounts/mod.rs`)
In accordance with `AGENTS.md` Rule 6, all new owner-keyed tables must be cataloged:
- Added to `ACCOUNT_ID_FOLLOWS`:
  - `"scheduled_agent_tasks"`
  - `"agent_runs"`
  - `"agent_approvals"`
*(Note: `agent_steps` cascades from `agent_runs` on account merge/deletion).*

---

## 5. Extensible Task Framework & Built-in Task Plugins [TODO Planned Extensions]

The runtime provides the `ScheduledAgentTask` trait and `ScheduledTaskRegistry` as the extensibility model. In the initial core implementation, **custom user goals (`!agent run <goal>`) and custom cron schedules** are fully functional end-to-end. Specific domain-specific built-in task plugins are deferred as **[TODO] planned incremental extensions**:

| Plugin Name | Default Cron | Whitelisted Tools | Functional Purpose | Status |
|---|---|---|---|---|
| `WeeklyBookmarkDigestTask` | `0 21 * * SUN` (Sunday 21:00 UTC) | `bookmarks_list`, `media_synthesize_speech` | Gathers bookmarks saved during the week, generates a multi-paragraph synthesis, triggers Kokoro TTS voice audio synthesis, and posts to user's chat channel. | **[TODO Extension]** |
| `MonthlyFinancialAuditTask` | `0 8 1 * *` (1st of month 08:00 UTC) | `finance_get_summary`, `finance_list_expenses` | Aggregates prior month spending across categories, detects budget drifts and recurring bill anomalies, and compiles recommendations. | **[TODO Extension]** |
| `PortfolioRebalanceAlertTask` | `0 9 * * MON` (Monday 09:00 UTC) | `portfolio_get_holdings`, `search_web` | Calculates holding weights against target allocations; alerts if any security has drifted by > 5%. | **[TODO Extension]** |
| `SystemMaintenanceAuditTask` | `0 3 * * *` (Daily 03:00 UTC) | *(Internal maintenance queries)* | Cleans expired agent approvals, purges dead link tokens, optimizes job history, and reports storage consumption. | **[TODO Extension]** |

---

## 6. Safety & Human-in-the-Loop Protocol

```text
       Step N: Model emits Tool Call -> "portfolio_execute_trade"
                                   │
                                   ▼
             Does ToolDefinition have is_sensitive == true?
                                   │
                 ┌─────────────────┴─────────────────┐
                 │ YES                               │ NO
                 ▼                                   ▼
    1. Create `agent_approvals` row             Execute tool directly
       (expires in 60 minutes)                  Record result in `agent_steps`
    2. Set `agent_runs.status = 'waiting_approval'`
    3. Publish `AgentApprovalRequired`
    4. Outbox delivers interactive prompt:
       "⚠️ Agent action requires approval:
        Trade: SELL 5.0 AAPL @ Market
        To approve: `!agent confirm <id>`
        To reject: `!agent reject <id>`"
    5. Job worker terminates current attempt (yields)
```

1. **Deterministic Halting:** A sensitive tool call never executes speculatively. The engine creates an approval row and yields the worker lease.
2. **Interactive Confirmation:** The user can review the exact parsed parameters in chat or on the web dashboard.
3. **Resumption:** When confirmed, the approval status is updated to `Approved`, the run is moved back to `Executing`, and a new `agent.execute_run` job is enqueued to resume the step immediately.
4. **Rejection & Graceful Fallback:** If rejected, the agent receives an `Observation: User rejected action '<tool_name>': <reason>` step and plans an alternative approach or terminates with an explanation.
5. **Expiry:** After 60 minutes, pending approvals expire automatically; the run marks the step failed and completes with an unapproved warning.

---

## 7. Verification Strategy & Test Matrix

In accordance with `AGENTS.md` §8:

| Test Area | What is Verified |
|---|---|
| **Domain Unit Tests** (`domain/agent/`) | Run state machine transitions, step sequence monotonic validation, approval expiry calculation, goal length restrictions, and parameter clamping. |
| **Repository Integration Tests** (`repositories/agent.rs`) | Round-trip persistence for runs, steps, and approvals; owner isolation (`assert_owner_isolation`); `FOR UPDATE SKIP LOCKED` concurrency on due tasks. |
| **Service Integration Tests** (`application/agent/`) | Single-active-run account constraint enforcement; transaction rollback mid-step; outbox event emissions (`AgentRunStarted`, `AgentStepCompleted`, `AgentRunCompleted`). |
| **LLM Sandboxing & Approval Tests** (`llm/agent/`) | Tool whitelist filtering; prompt generation; sensitive tool interception into `waiting_approval`; token accumulation accuracy. |
| **Job Handler Tests** (`interface/jobs/agent.rs`) | Worker execution loop; handling yields on approval; graceful exit on cancel; error truncation on step failure. |
| **Architecture Invariants** (`tests/architecture.rs`) | `agent` domain purity (no sqlx, no axum, no matrix-sdk); `agent` job handlers are interface code; `OWNED_TABLES` catalog completeness. |
| **Contract Stability** | `backend/openapi.json` checked for exact consistency and new DTO registration. |

---

## 8. Rollout Plan

1. **Step 1:** Migration `0010_agent_runtime.sql` and `domain/agent/` domain types + `OWNED_TABLES` catalog registration.
2. **Step 2:** `infrastructure/postgres/repositories/agent.rs` with full CRUD and concurrency tests.
3. **Step 3:** `application/agent/` (`AgentService`) and `SystemEvent` definitions.
4. **Step 4:** `llm/agent/` ReAct execution engine, tool sandboxing, and token accounting.
5. **Step 5:** `interface/jobs/agent.rs` durable job worker handler (`agent.execute_run`) and scheduler hook.
6. **Step 6:** `interface/chat/agent.rs` (`!agent` chat module) and `interface/http/agent.rs` REST endpoints.
7. **Step 7:** Full verification gates (`cargo fmt`, `check`, `clippy -D warnings`, `test`, `architecture`), `openapi.json` sync, and documentation update.
8. **Subsequent Milestones:** Implement built-in task plugins (`WeeklyBookmarkDigestTask`, `MonthlyFinancialAuditTask`, etc.) as incremental plugins atop the established framework.

