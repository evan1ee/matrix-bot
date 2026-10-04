# Technical Debt Remediation Design

**Date:** 2026-10-04  
**Status:** Approved  
**Context:** [`SPEC.md`](../../SPEC.md), [`docs/superpowers/HANDOFF.md`](../HANDOFF.md) (§11 Deferred items), [`AGENTS.md`](../../AGENTS.md)  
**Target Branch:** `feat/tech-debt-remediation`

---

## 1. Problem Statement & Motivation

Following the completion of the 4 core sub-projects (PostgreSQL Foundation, Omnichannel, Storage & Autonomous Agent Runtime, and Web Console), an audit of the codebase identified key technical debts:

1. **Security — Re-link Cross-Account Data Merge**:
   In [`application/identity/link.rs`](file:///d:/01_Data/code/matrix-hello-bot/backend/src/application/identity/link.rs), when an identity previously unlinked from Account A is linked by Account B, `previous_account_with_data` triggers a full data merge from Account A into Account B if Account A still has data. If Account A intentionally unlinked a secondary channel, Account B unintentionally inherits Account A's private domain state.
2. **Architecture — 500-Line Production Limit Violations**:
   [`AGENTS.md`](file:///d:/01_Data/code/matrix-hello-bot/AGENTS.md) §4.3 mandates: *"No file in `backend/src/` or `inference/src/` exceeds 500 lines of production code (target < 300; `#[cfg(test)]` modules do not count)."*
   The Autonomous Agent subsystem introduced three files exceeding this threshold:
   - `backend/src/domain/agent/mod.rs` (~680 prod lines)
   - `backend/src/infrastructure/postgres/repositories/agent.rs` (~648 prod lines)
   - `backend/src/application/agent/mod.rs` (~621 prod lines)
3. **Architecture — Domain Layer Purity Breaches**:
   `backend/src/domain/accounts/session.rs` imports `sha2`, `webauthn_rs::prelude::Passkey`, `serde_json`, and `tracing::error!`. The domain layer must depend only on `std`, `chrono`, `uuid`, `serde`, and `thiserror`.
4. **Reliability — Delivery Channel Awareness**:
   `ChannelSink::is_ready` returns true if *any* channel is connected. If Discord is down while Matrix is up, pending Discord outbox deliveries are claimed, fail with connection errors, and burn retry attempts into exponential backoff.
5. **Reliability — Telegram Outbox Deduplication**:
   Telegram Bot API lacks idempotency keys. Outbox retries or worker restarts post-send can deliver duplicate notifications.
6. **LLM — ReAct Tool Output Prompt Scaling**:
   During 15-step agent runs, accumulating large raw JSON outputs in prompt history risks token exhaustion.
7. **Frontend — Monolithic Components & Accessibility**:
   `frontend/src/modules/portfolio/index.tsx` (1,248 lines) and `frontend/src/modules/finance/index.tsx` (807 lines) conflate state, modal workflows, and presentation. Additionally, `CustomSelect.tsx` lacks WAI-ARIA combobox accessibility.

---

## 2. Settled Decisions (R165–R171)

Through user consultation and codebase alignment, the following rulings are settled:

- **R165 (Re-Link Merge Guard)**: Automatic merges on identity re-linking are strictly restricted to unclaimed holder accounts (`STATUS_UNCLAIMED`). Re-linking an identity previously unlinked from an active account (`STATUS_ACTIVE`) is treated as a clean fresh bind (`LinkOutcome::Linked`) with zero data absorbed from the prior account.
- **R166 (Per-Channel Delivery Readiness)**: Outbox delivery dispatch checks destination channel connectivity. Offline channel deliveries are skipped without claiming or burning attempts, resuming immediately upon adapter reconnect.
- **R167 (Agent Subsystem File Decomposition)**: Monolithic agent files (`domain/agent/mod.rs`, `repositories/agent.rs`, `application/agent/mod.rs`) are decomposed into cohesive functional submodules (`run`, `step`, `approval`, `config`/`task`) < 300 lines each.
- **R168 (Telegram Delivery Deduplication)**: In-memory 60-second sliding-window deduplication in `TelegramAdapter` / `outbound.rs` suppresses duplicate message transmissions from worker retries or re-claimed leases.
- **R169 (ReAct Tool Output Budgeting)**: Tool outputs exceeding 2,000 characters in ReAct prompt history are truncated with an explicit note, while preserving full unabridged JSON in PostgreSQL `agent_steps`.
- **R170 (Frontend Monolithic Decomposition Scope)**: Decompose the two largest outliers `portfolio/index.tsx` (1,248 lines) and `finance/index.tsx` (807 lines) into modular subcomponents (< 350 lines each), deferring the ~600-line modules (`todo`, `mood`, `reminder`) to keep this sprint focused and low-risk.
- **R171 (Frontend Test Toolchain)**: Rely on the existing 199 Playwright E2E suite for verification without introducing Vitest or extra dependencies in this sprint.

---

## 3. Architecture & Technical Design

### 3.1 Security: Guarding Re-Link Merges

In `backend/src/application/identity/link.rs`:
- In `previous_account_with_data`:
  ```rust
  let (status,): (String,) = sqlx::query_as("SELECT status FROM accounts WHERE id = $1")
      .bind(previous.as_uuid())
      .fetch_one(&mut *conn)
      .await?;
  if status != STATUS_UNCLAIMED {
      // Active accounts are never absorbed automatically by a re-linking identity.
      return Ok(None);
  }
  ```
- This ensures an unlinked channel identity can be rebound to a new account cleanly as `LinkOutcome::Linked` without extracting prior account data.

### 3.2 Domain Layer Purity in `domain/accounts`

- In `backend/src/domain/accounts/session.rs`:
  - Remove `webauthn_rs::prelude::Passkey` import.
  - Remove `StoredCredential::passkey(&self) -> Option<webauthn_rs::prelude::Passkey>`.
  - Replace `hash_token` implementation using a pure helper in `crate::utils::secure` or abstract it out of domain.
- In `backend/src/application/accounts/passkeys.rs`:
  - Add helper function `pub fn parse_passkey(credential: &StoredCredential) -> Result<Passkey, AppError>` inside the application layer where `webauthn_rs` belongs.
- In `backend/tests/architecture.rs`:
  - Extend `domain_depends_on_nothing_outside_itself` to explicitly forbid `webauthn_rs` and `sha2` in `domain/`.

### 3.3 Decomposing the Agent Subsystem

Decompose the large files along natural responsibility seams:

1. **`domain/agent/`**:
   - `mod.rs`: re-exports, constants (`MAX_GOAL_CHARS`, `DEFAULT_MAX_STEPS`), `TaskId`, `AgentError`.
   - `run.rs`: `AgentRunId`, `AgentRunStatus`, `AgentRun`.
   - `step.rs`: `AgentStepId`, `AgentStepType`, `AgentStepStatus`, `AgentStep`.
   - `approval.rs`: `AgentApprovalId`, `ApprovalStatus`, `AgentApproval`.
   - `config.rs`: `TaskSchedule`, `AgentTaskConfig`.
   - `task.rs`: `ScheduledAgentTask`, `ScheduledTaskRegistry` (existing).
2. **`infrastructure/postgres/repositories/agent/`**:
   - `mod.rs`: module declarations and re-exports.
   - `tasks.rs`: scheduled task configs and schedule queries.
   - `runs.rs`: agent run creation, status transitions, run queries.
   - `steps.rs`: granular ReAct execution step logging.
   - `approvals.rs`: human-in-the-loop approval requests and resolutions.
3. **`application/agent/`**:
   - `mod.rs`: `AgentService` struct definition, wiring, public API surface.
   - `lifecycle.rs`: starting runs, advancing execution steps, finishing runs.
   - `approvals.rs`: requesting, approving, rejecting, and resuming paused runs.
   - `tasks.rs`: scheduling, updating cron expressions, trigger sweeps.

Each resulting file stays strictly under 300 lines of production code.

### 3.4 Per-Channel Delivery Readiness in `ChannelSink`

- Update `ChannelSink::can_deliver(&self, destination: &Destination) -> bool`:
  - Checks `registry.adapter(destination.channel()).is_some_and(|a| a.is_connected())`.
- In `events/dispatcher.rs`:
  - When evaluating claimed deliveries, verify `can_deliver`. If target adapter is offline, do not burn attempt counter; defer until adapter reconnection.

### 3.5 Telegram Outbound Deduplication

- In `backend/src/interface/channels/telegram/outbound.rs`:
  - Introduce an in-memory sliding deduplication cache `TelegramSendDeduper` wrapped in `Arc<Mutex<_>>`.
  - Keys on `(chat_id, sha256(text))` with a 60-second TTL.
  - If a worker claims an outbox delivery that was already transmitted within 60 seconds (due to network timeout on the worker side after Telegram accepted the message), the duplicate send is suppressed and returns success immediately.

### 3.6 ReAct Engine Tool Output Budgeting

- In `backend/src/llm/agent/engine.rs`:
  - Enforce `const MAX_TOOL_OUTPUT_PROMPT_CHARS: usize = 2000;`.
  - When formatting `AgentStep` history for the ReAct LLM prompt:
    - If `step.tool_output` string representation exceeds `MAX_TOOL_OUTPUT_PROMPT_CHARS`, truncate with `... [output truncated for prompt length; full output stored in step record]`.

### 3.7 Frontend Modularization & CustomSelect Accessibility

1. **Portfolio Component Split**:
   - `frontend/src/modules/portfolio/components/HoldingTable.tsx`: table listing, sorting, and inline display.
   - `frontend/src/modules/portfolio/components/SellModal.tsx`: sell dialog and quantity validation.
   - `frontend/src/modules/portfolio/components/BuyModal.tsx`: asset purchase and ticker search.
   - `frontend/src/modules/portfolio/components/CashCard.tsx`: currency balance management.
   - `frontend/src/modules/portfolio/index.tsx`: reduced to top-level view orchestration (< 350 lines).
2. **Finance Component Split**:
   - `frontend/src/modules/finance/components/TransactionList.tsx`: transaction table & filters.
   - `frontend/src/modules/finance/components/AddTransactionModal.tsx`: expense/income entry form.
   - `frontend/src/modules/finance/index.tsx`: reduced to top-level view orchestration (< 350 lines).
3. **CustomSelect WAI-ARIA**:
   - Add `role="combobox"`, `aria-expanded={isOpen}`, `aria-haspopup="listbox"`, `aria-controls`.
   - Popover gets `role="listbox"`, options get `role="option"`, `aria-selected`.
   - Add keyboard event handling for `ArrowDown`, `ArrowUp`, `Enter`, and `Escape`.

---

## 4. Verification & Acceptance Criteria

1. **Security Test**:
   - Unit test in `backend/src/application/identity/link.rs` verifying that an unlinked identity from an active account does not merge data into another account.
2. **Architecture Guards**:
   - `backend/tests/architecture.rs`:
     - Asserts `webauthn_rs` and `sha2` do not exist in `domain/`.
     - Asserts all files in `backend/src/` remain under 500 lines of production code.
3. **Workspace Checks**:
   - `cargo clippy --workspace --all-targets -- -D warnings`: 0 warnings.
   - `cargo test -p matrix-bot --all-targets`: 100% green.
   - `cargo test -p matrix-bot --test architecture`: all tests pass.
   - `npm --prefix frontend run build`: 0 build errors.
   - `npm --prefix frontend run test:e2e`: 199/199 Playwright E2E tests green.
   - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`: byte-identical.
