# Implementation Plan: Deep Procedural Pipelines for Built-in Agent Tasks

**Goal:** Implement specialized procedural workflows for all 5 built-in agent task plugins in `matrix-bot`: deterministic mathematical engines in `domain/`, analytical agent tools in `llm/tools/`, enhanced goal formulations, and direct Kokoro TTS audio compilation for weekly bookmark digests.

**Specification:** [`docs/superpowers/specs/2026-10-18-deep-agent-pipelines.md`](../specs/2026-10-18-deep-agent-pipelines.md)

---

## Task Breakdown

### Task 1: Pure Domain Calculation Engines

**Files:**
- Create: `backend/src/domain/portfolio/rebalance.rs`
- Modify: `backend/src/domain/portfolio/mod.rs`
- Create: `backend/src/domain/finance/audit.rs`
- Modify: `backend/src/domain/finance/mod.rs`
- Create: `backend/src/domain/recurring_bills/due.rs`
- Modify: `backend/src/domain/recurring_bills/mod.rs`
- Create: `backend/src/domain/habits/consistency.rs`
- Modify: `backend/src/domain/habits/mod.rs`

**Interfaces:**
- Consumes: `Holding`, `Transaction`, `RecurringBill`, `HabitWithStatus`
- Produces:
  - `evaluate_portfolio_drift(holdings, targets, threshold_pct) -> PortfolioDriftReport`
  - `evaluate_monthly_audit(transactions, recurring_bills, threshold_pct) -> MonthlyAuditReport`
  - `evaluate_bill_cash_adequacy(bills, cash_balances, today, days_ahead) -> CashFlowAdequacyReport`
  - `evaluate_habit_consistency(habits) -> HabitConsistencyReport`

- [ ] **Step 1: Write unit tests for pure domain functions (TDD RED)**
- [ ] **Step 2: Implement domain data structures and pure functions (TDD GREEN)**
- [ ] **Step 3: Verify all unit tests pass**
  - Run: `cargo test -p matrix-bot domain::portfolio::rebalance domain::finance::audit domain::recurring_bills::due domain::habits::consistency`
- [ ] **Step 4: Commit**
  - Message: `feat(domain): implement deterministic calculation engines for agent task pipelines`

---

### Task 2: Specialized Analytical Agent Tools

**Files:**
- Create: `backend/src/llm/tools/portfolio/rebalance.rs`
- Modify: `backend/src/llm/tools/portfolio/mod.rs`
- Modify: `backend/src/llm/tools/finance.rs`
- Modify: `backend/src/llm/tools/bills.rs`
- Modify: `backend/src/llm/tools/habits.rs`
- Modify: `backend/src/llm/tools/definitions.rs`
- Modify: `backend/src/llm/tools/executor.rs`

**Interfaces:**
- Produces:
  - Tool `evaluate_portfolio_rebalance`: evaluates targets vs current holdings.
  - Tool `audit_monthly_finances`: evaluates 30-day spending spikes and recurring commitments.
  - Tool `evaluate_bill_due_adequacy`: evaluates upcoming bills against cash balances.
  - Tool `evaluate_habit_consistency`: evaluates weekly consistency and streaks.

- [ ] **Step 1: Write unit tests for analytical tools (TDD RED)**
- [ ] **Step 2: Implement tool executors and OpenAI function definitions (TDD GREEN)**
- [ ] **Step 3: Verify tools execute properly**
  - Run: `cargo test -p matrix-bot llm::tools`
- [ ] **Step 4: Commit**
  - Message: `feat(llm): expose specialized analytical tools for agent task pipelines`

---

### Task 3: Built-in Task Plugins & Goal Formulations

**Files:**
- Modify: `backend/src/domain/agent/task.rs`
- Tests: `backend/src/domain/agent/task.rs` test module

**Interfaces:**
- Consumes: analytical tool names, config JSON
- Produces:
  - Updated `allowed_tools` and `formulate_goal` across all 5 plugins:
    - `WeeklyBookmarkDigestTask`
    - `MonthlyFinancialAuditTask`
    - `PortfolioRebalanceAlertTask`
    - `BillDueDigestTask`
    - `HabitConsistencyReviewTask`

- [ ] **Step 1: Write tests for task whitelists and goal formulation (TDD RED)**
- [ ] **Step 2: Implement updated plugins in `domain/agent/task.rs` (TDD GREEN)**
- [ ] **Step 3: Verify tests pass and file stays under 500 lines**
  - Run: `cargo test -p matrix-bot domain::agent::task`
- [ ] **Step 4: Commit**
  - Message: `feat(agent): enhance built-in task plugins with deep procedural pipelines`

---

### Task 4: Direct Kokoro TTS Audio Digest Compilation

**Files:**
- Modify: `backend/src/interface/jobs/agent/mod.rs`
- Modify: `backend/src/interface/jobs/agent/resume.rs` (if applicable)
- Tests: unit and integration tests in `backend/src/interface/jobs/agent/`

**Interfaces:**
- Consumes: `<audio_script>...</audio_script>`, `TtsClient::synthesize`, `OutgoingMessage`
- Produces:
  - Audio script extraction helper.
  - TTS synthesis of the audio script for `WeeklyBookmarkDigestTask`.
  - Attachment of synthesized Ogg Opus audio (`AttachmentKind::Audio`) to `OutgoingMessage`.
  - Clean fallback to text-only if TTS fails or is disabled.

- [ ] **Step 1: Write tests for audio script extraction and attachment compilation (TDD RED)**
- [ ] **Step 2: Implement audio digest compilation in `AgentExecuteRunHandler` (TDD GREEN)**
- [ ] **Step 3: Verify tests pass**
  - Run: `cargo test -p matrix-bot interface::jobs::agent`
- [ ] **Step 4: Commit**
  - Message: `feat(agent): compile Kokoro TTS audio digests for weekly bookmark digests`

---

### Task 5: Documentation & Hand-Off Update

**Files:**
- Modify: `docs/superpowers/HANDOFF.md`

- [ ] **Step 1: Update Track 2 item 2 in `HANDOFF.md` to `(Fixed)` with design rulings**
- [ ] **Step 2: Commit**
  - Message: `docs(superpowers): resolve deep procedural agent pipelines in handoff`

---

## Global Verification Gates

Before completing the feature:
1. `cargo test -p matrix-bot` (all tests passing)
2. `cargo test -p matrix-bot --test architecture` (all 39 passing)
3. `cargo clippy -p matrix-bot --lib --tests -- -D warnings` (0 warnings)
4. Verify production file lengths: every modified file < 500 production lines.
