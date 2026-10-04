# Plan: Cutover Staging Rehearsal, Edge-Case Resilience, and Frontend Polish

> **Branch:** `feat/cutover-resilience-frontend-polish`  
> **Scope:** Option 2 (Cutover runbook & staging rehearsal), Option 3 (Edge-case recovery & reminder resilience, E2E link tests), Option 4 (Decompose remaining frontend modules >500 lines: `todo`, `mood`, `reminder`).  
> **Authority:** `AGENTS.md`, `SPEC.md`, `docs/superpowers/HANDOFF.md`.

---

## 1. Objectives & Quality Gates

1. **Production Cutover Runbook & Staging Rehearsal (Option 2)**:
   - Bring `docs/superpowers/runbooks/postgres-foundation-cutover.md` fully up to date with all migrations through `0010_agent_runtime.sql`.
   - Add verification procedures for storage (`media.process_receipt`), agent runtime (`agent.execute_run`), and all 4 chat channels (Matrix, Discord, Telegram, Email).
   - Document a definitive step-by-step production cutover checklist.

2. **Reliability & Edge-Case Resilience (Option 3)**:
   - In `interface/chat/account.rs`, update `redeem_matured_recovery` to expose `Database(AppError)` explicitly.
   - In `interface/http/auth/recovery.rs`, return HTTP 500 when `redeem_matured_recovery` hits a database error, rather than masking it as 401 Unauthorized.
   - In `interface/jobs/reminders.rs`, prevent recurring reminders from being permanently marked `failed` if delivery already succeeded and only the database advance step encountered a transient error.
   - In `frontend/e2e/`, add Playwright E2E coverage for `/link` edge cases (sign-in-required, re-inspect round-trip, 403 step-up passkey assertion).

3. **Frontend Module Decomposition (Option 4)**:
   - Decompose `frontend/src/modules/todo/index.tsx` (599 lines) into modular subcomponents (< 300 lines each).
   - Decompose `frontend/src/modules/mood/index.tsx` (578 lines) into modular subcomponents (< 300 lines each).
   - Decompose `frontend/src/modules/reminder/index.tsx` (554 lines) into modular subcomponents (< 300 lines each).
   - Ensure 100% of frontend files adhere strictly to the < 500 lines rule in `AGENTS.md` §4.3.

4. **Verification Gates**:
   - `cargo fmt --all -- --check`
   - `cargo clippy --workspace --all-targets -- -D warnings`
   - `cargo test -p matrix-bot --test architecture`
   - `cargo test -p matrix-bot --lib -- --test-threads=4`
   - `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json` (byte-identical)
   - `npm --prefix frontend run build` (zero TS/bundle errors)
   - `npm --prefix frontend run test:e2e` (100% green across all Playwright specs)

---

## 2. Tasks & Execution Steps

### Task 1: Update Production Cutover Runbook (Option 2)
- **Files**: `docs/superpowers/runbooks/postgres-foundation-cutover.md`
- **Actions**:
  1. Add migration guidance for `0009_receipt_storage_key.sql` and `0010_agent_runtime.sql`.
  2. Document environment variables for omnichannel, storage (`STORAGE_BACKEND`, `STORAGE_LOCAL_ROOT`, `STORAGE_S3_*`), and autonomous agent.
  3. Detail pre-flight verification checks for background worker queues, advisory locks, and multi-channel connectivity.
  4. Commit: `docs: update production cutover runbook for migrations 0009-0010, storage, and agent runtime`.

### Task 2: Fix Recovery Error Mapping (Option 3a)
- **Files**:
  - `backend/src/interface/chat/account.rs`
  - `backend/src/interface/http/auth/recovery.rs`
- **Actions**:
  1. Update `RecoveryRejection` in `interface/chat/account.rs` to include `Database(crate::application::AppError)` or return `Result<AccountId, RecoveryError>`.
  2. In `interface/http/auth/recovery.rs`, map `Database` errors to `ApiError::internal("Database error during recovery check")` (HTTP 500).
  3. Add test verifying database errors propagate as 500 while invalid/expired codes return 401.
  4. Commit: `fix(auth): return 500 on database error during recovery code redemption`.

### Task 3: Safeguard Recurring Reminders on Post-Delivery DB Errors (Option 3b)
- **Files**:
  - `backend/src/interface/jobs/reminders.rs`
  - `backend/src/application/reminders/delivery.rs`
- **Actions**:
  1. In `ReminderFireHandler::fire`, track whether `sent` completed successfully (`sent_ok = true`).
  2. If `sent` succeeded but `complete_delivery` fails, return a distinct error or avoid calling `mark_failed_at` for recurring reminders on `ctx.is_last_attempt()`.
  3. Add test asserting a recurring reminder with a transient database error during advance is not marked `failed`.
  4. Commit: `fix(reminders): preserve recurring schedule when post-delivery advance encounters database error`.

### Task 4: Add E2E Coverage for LinkPage Edge Cases (Option 3c)
- **Files**:
  - `frontend/e2e/link.spec.ts` (or update `screens.spec.ts` / `security.spec.ts`)
  - `frontend/e2e/fake-api.ts`
- **Actions**:
  1. In `fake-api.ts`, ensure `/link/inspect` supports `SignInRequired` and `/link/claim` mock responses for 403 step-up.
  2. Add Playwright test covering `/link?token=test` when sign-in is required, clicking "Sign in with passkey", and re-inspecting.
  3. Commit: `test(e2e): add Playwright coverage for LinkPage sign-in required and step-up flows`.

### Task 5: Decompose `frontend/src/modules/todo/index.tsx` (Option 4a)
- **Files**:
  - `frontend/src/modules/todo/index.tsx`
  - `frontend/src/modules/todo/components/TodoItemRow.tsx`
  - `frontend/src/modules/todo/components/TodoToolbar.tsx`
- **Actions**:
  1. Extract item rendering, editing, priority badge, and reminder status into `TodoItemRow.tsx`.
  2. Extract search input, priority filters, and new task entry into `TodoToolbar.tsx`.
  3. Reduce `index.tsx` to orchestration under 250 lines.
  4. Verify with `npx playwright test e2e/todos.spec.ts e2e/todo-due-reminder.spec.ts`.
  5. Commit: `refactor(frontend): decompose todo module into subcomponents (<300 lines)`.

### Task 6: Decompose `frontend/src/modules/mood/index.tsx` (Option 4b)
- **Files**:
  - `frontend/src/modules/mood/index.tsx`
  - `frontend/src/modules/mood/components/MoodScoreSelector.tsx`
  - `frontend/src/modules/mood/components/MoodHistoryList.tsx`
- **Actions**:
  1. Extract mood grid selector, valence/energy inputs, and notes into `MoodScoreSelector.tsx`.
  2. Extract history list, reflections, and tag filter pills into `MoodHistoryList.tsx`.
  3. Reduce `index.tsx` to orchestration under 250 lines.
  4. Verify with `npx playwright test e2e/mood.spec.ts e2e/time-blocks.spec.ts`.
  5. Commit: `refactor(frontend): decompose mood module into subcomponents (<300 lines)`.

### Task 7: Decompose `frontend/src/modules/reminder/index.tsx` (Option 4c)
- **Files**:
  - `frontend/src/modules/reminder/index.tsx`
  - `frontend/src/modules/reminder/components/ReminderList.tsx`
  - `frontend/src/modules/reminder/components/AddReminderModal.tsx`
- **Actions**:
  1. Extract active/recurring reminder lists and action triggers into `ReminderList.tsx`.
  2. Extract reminder creation modal, date/time pickers, and recurrence select into `AddReminderModal.tsx`.
  3. Reduce `index.tsx` to orchestration under 250 lines.
  4. Verify with `npx playwright test e2e/reminders.spec.ts`.
  5. Commit: `refactor(frontend): decompose reminder module into subcomponents (<300 lines)`.

### Task 8: Full Verification & Documentation
- **Actions**:
  1. Run full Rust gates: `cargo fmt`, `cargo clippy`, `cargo test -p matrix-bot --test architecture`, `cargo test -p matrix-bot --lib -- --test-threads=4`.
  2. Verify OpenAPI contract: `cargo run --bin generate-openapi && git diff --exit-code backend/openapi.json`.
  3. Run full frontend build and E2E test suite: `npm --prefix frontend run build`, `npm --prefix frontend run test:e2e`.
  4. Update `docs/superpowers/HANDOFF.md` §4 and §11.
  5. Commit: `docs: update HANDOFF.md with cutover, resilience, and frontend decomposition results`.
