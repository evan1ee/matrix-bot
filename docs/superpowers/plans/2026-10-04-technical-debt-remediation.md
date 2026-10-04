# Technical Debt Remediation Implementation Plan

**Date:** 2026-10-04  
**Status:** Ready for execution  
**Design:** [`docs/superpowers/specs/2026-10-04-technical-debt-remediation-design.md`](file:///d:/01_Data/code/matrix-hello-bot/docs/superpowers/specs/2026-10-04-technical-debt-remediation-design.md)  
**Target Branch:** `feat/tech-debt-remediation` (branched from `master`)

---

## Task Breakdown

### Task 1: Re-Link Cross-Account Merge Security Guard (R165)
- [ ] Edit `backend/src/application/identity/link.rs`:
  - In `previous_account_with_data`:
    - Query the status of `previous` account (`SELECT status FROM accounts WHERE id = $1`).
    - If status is NOT `STATUS_UNCLAIMED`, return `Ok(None)`: active accounts with their own passkeys/credentials must never be absorbed via a simple chat token link from another account.
- [ ] Add unit/service test in `backend/src/application/identity/link.rs`:
  - Setup: Create Account A (active, with todo/finance data). Bind identity `matrix:@user:homeserver` to Account A.
  - Action: Unlink the identity from Account A. Create Account B (active). Start link token for `@user:homeserver` and claim on Account B.
  - Assert:
    - Outcome is `LinkOutcome::Linked` (NOT `LinkOutcome::Merged`).
    - Account A's todos/finance records remain exclusively on Account A.
    - Account B owns only its own data.
- [ ] Verify: `cargo test -p matrix-bot --lib application::identity::link`.

---

### Task 2: Domain Layer Purity in `domain/accounts`
- [ ] Edit `backend/src/domain/accounts/session.rs`:
  - Remove `use sha2::{Digest, Sha256};` and `use webauthn_rs::prelude::Passkey;`.
  - Remove `StoredCredential::passkey(&self) -> Option<Passkey>`.
  - In `hash_token`, delegate to `crate::utils::secure::sha256_hex` or pure std formatting.
- [ ] Edit `backend/src/application/accounts/passkeys.rs`:
  - Implement `pub fn parse_passkey(credential: &StoredCredential) -> Result<Passkey, AppError>` here in the application layer.
  - Update callers in `application/accounts/` and `interface/http/auth/webauthn.rs` to use `parse_passkey(&credential)` instead of the domain method.
- [ ] Edit `backend/tests/architecture.rs`:
  - Add `"webauthn_rs"` and `"sha2"` to the forbidden dependencies list in `domain_depends_on_nothing_outside_itself`.
- [ ] Verify:
  - `cargo test -p matrix-bot --test architecture`
  - `cargo test -p matrix-bot --lib domain::accounts`
  - `cargo test -p matrix-bot --lib application::accounts`

---

### Task 3: Autonomous Agent Subsystem Decomposition (<500 Production Lines, R167)
- [ ] Decompose `backend/src/domain/agent/`:
  - Create `domain/agent/run.rs`: `AgentRunId`, `AgentRunStatus`, `AgentRun`.
  - Create `domain/agent/step.rs`: `AgentStepId`, `AgentStepType`, `AgentStepStatus`, `AgentStep`.
  - Create `domain/agent/approval.rs`: `AgentApprovalId`, `ApprovalStatus`, `AgentApproval`.
  - Create `domain/agent/config.rs`: `TaskSchedule`, `AgentTaskConfig`, constants.
  - Update `domain/agent/mod.rs` to re-export types; verify it contains < 100 production lines.
- [ ] Decompose `backend/src/infrastructure/postgres/repositories/agent/`:
  - Convert `repositories/agent.rs` into a directory module `repositories/agent/`:
    - `tasks.rs`: scheduled tasks CRUD and due query.
    - `runs.rs`: agent run creation, status transitions, queries.
    - `steps.rs`: granular step logging.
    - `approvals.rs`: approval creation and resolution.
    - `mod.rs`: re-exports matching the existing repository API.
- [ ] Decompose `backend/src/application/agent/`:
  - Create `application/agent/lifecycle.rs`: starting runs, step loop orchestration, completion.
  - Create `application/agent/approvals.rs`: approval verification, resume logic.
  - Create `application/agent/tasks.rs`: scheduled task management.
  - Update `application/agent/mod.rs` to re-export `AgentService` and maintain the public API.
- [ ] Add architecture test in `backend/tests/architecture.rs`:
  - `agent_files_satisfy_line_count_limit`: verify no file in `domain/agent/`, `repositories/agent/`, or `application/agent/` exceeds 500 lines.
- [ ] Verify:
  - `cargo test -p matrix-bot --test architecture`
  - `cargo test -p matrix-bot --lib application::agent`
  - `cargo test -p matrix-bot --lib domain::agent`

---

### Task 4: Channel-Aware Delivery Readiness in `ChannelSink` (R166)
- [ ] Edit `backend/src/interface/events/mod.rs`:
  - In `ChannelSink`:
    - Add `pub fn can_deliver(&self, destination: &Destination) -> bool`.
    - Looks up the adapter by `destination.channel()` from `self.registry`.
    - Returns `adapter.is_connected()`.
- [ ] Edit outbox delivery loop / dispatcher (`events/dispatcher.rs`):
  - When evaluating claimed deliveries, verify `can_deliver`.
  - If the target channel adapter is disconnected, release or defer the delivery without incrementing `attempts`.
- [ ] Add unit test in `backend/src/events/dispatcher.rs`:
  - Simulate an outbox delivery for a channel whose adapter is disconnected; assert attempt count is not burned.
- [ ] Verify: `cargo test -p matrix-bot --lib events`.

---

### Task 5: Telegram Outbox Message Deduplication Window (R168)
- [ ] Edit `backend/src/interface/channels/telegram/outbound.rs`:
  - Add `TelegramSendDeduper`:
    - Keeps a thread-safe cache (`Arc<Mutex<HashMap<(i64, String), Instant>>>`).
    - Purges entries older than 60 seconds.
    - `check_and_record(chat_id: i64, content_hash: &str) -> bool`: returns `false` if sent within the last 60 seconds.
  - In `TelegramOutbound::send`:
    - If `check_and_record` detects a duplicate within 60s, log `info!("Telegram send suppressed by deduplication window")` and return `Ok(())`.
- [ ] Add unit test in `backend/src/interface/channels/telegram/outbound.rs`:
  - Assert that calling `send` with identical chat and body twice within 100ms sends only one upstream request.
- [ ] Verify: `cargo test -p matrix-bot --lib interface::channels::telegram`.

---

### Task 6: ReAct Tool Output Token Budgeting (R169)
- [ ] Edit `backend/src/llm/agent/engine.rs`:
  - Introduce `const MAX_TOOL_OUTPUT_PROMPT_CHARS: usize = 2000;`.
  - When formatting `AgentStep` history for the ReAct LLM prompt:
    - If `step.tool_output` string representation exceeds `MAX_TOOL_OUTPUT_PROMPT_CHARS`, truncate with `... [output truncated for prompt length; full output stored in step record]`.
- [ ] Add unit test in `backend/src/llm/agent/engine.rs`:
  - Assert prompt builder truncates oversized tool outputs while preserving head and footer context.
- [ ] Verify: `cargo test -p matrix-bot --lib llm::agent`.

---

### Task 7: Frontend Portfolio & Finance Module Decomposition (R170)
- [ ] In `frontend/src/modules/portfolio/`:
  - Extract `components/HoldingTable.tsx`: holds holdings table rendering and sort filters.
  - Extract `components/SellModal.tsx`: holds sell modal state, form validation, and submit action.
  - Extract `components/BuyModal.tsx`: asset purchase and ticker search.
  - Extract `components/CashCard.tsx`: holds cash balance presentation and deposit/withdraw triggers.
  - Refactor `index.tsx`: assemble the components; verify file is under 350 lines.
- [ ] In `frontend/src/modules/finance/`:
  - Extract `components/TransactionList.tsx`: transaction table & filters.
  - Extract `components/AddTransactionModal.tsx`: expense/income entry form.
  - Refactor `index.tsx`: assemble components; verify file is under 350 lines.
- [ ] Verify:
  - `npm --prefix frontend run build` (zero type errors).
  - `npm --prefix frontend run test:e2e` (portfolio and finance specs remain green).

---

### Task 8: CustomSelect WAI-ARIA Accessibility
- [ ] Edit `frontend/src/components/CustomSelect.tsx`:
  - Add `role="combobox"`, `aria-expanded={isOpen}`, `aria-haspopup="listbox"`, `aria-controls={listboxId}` to trigger button.
  - Add `role="listbox"`, `id={listboxId}` to dropdown popover.
  - Add `role="option"`, `aria-selected={isSelected}` to each option button.
  - Add `onKeyDown` to handle keyboard navigation (`ArrowDown`, `ArrowUp`, `Enter`, `Escape`).
- [ ] Verify:
  - `npm --prefix frontend run build`
  - `npm --prefix frontend run test:e2e` (all specs pass without interaction interception).

---

### Task 9: Final Full Verification & Documentation (R171)
- [ ] Run full project gates:
  - `cargo fmt --all -- --check`
  - `cargo check -p matrix-bot --all-targets`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test -p matrix-bot --all-targets`
  - `cargo test -p matrix-bot --test architecture`
  - `npm --prefix frontend run build`
  - `npm --prefix frontend run test:e2e` (199/199 green)
  - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`
- [ ] Update `docs/superpowers/HANDOFF.md`:
  - Record progress, rulings (R165–R171), and clear resolved items in §11.
