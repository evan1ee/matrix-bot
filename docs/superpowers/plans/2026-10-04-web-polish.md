# Sub-project 4 — Web Console & Multi-Interface Polish Plan

**Status:** Ready to execute.  
**Design:** [`docs/superpowers/specs/2026-10-04-web-polish-design.md`](file:///d:/01_Data/code/matrix-hello-bot/docs/superpowers/specs/2026-10-04-web-polish-design.md) (approved 2026-10-04).  
**Roadmap:** `SPEC.md` Phase 6 (§14.4 & §14.5).  
**Target Branch:** `feat/web-polish` (branched from `master`).  

---

## Task Breakdown

### Task 1: Fix `CustomSelect` Popover Interception & Stabilize E2E Baseline
- [x] Inspect and fix `frontend/src/components/CustomSelect.tsx`:
  - Ensure selecting an option immediately detaches the popover DOM / closes without blocking pending pointer events.
  - Check pointer-events / overlay styling during exit transitions.
- [x] Update `frontend/e2e/fixtures.ts`:
  - In `chooseOption` and `chooseCurrency`, ensure the popover is waited for until closed/detached before returning to the caller.
- [x] Run Playwright tests on the 4 failing specs:
  - `npx playwright test frontend/e2e/ai-chat.spec.ts frontend/e2e/finance.spec.ts frontend/e2e/screens.spec.ts frontend/e2e/todo-due-reminder.spec.ts`.
- [x] Verify 100% green baseline on all pre-existing tests (193/193 passed).

---

### Task 2: Reactive SSE Sync & React Query Cache Invalidation
- [x] Update `frontend/src/core/useSystemEvents.ts`:
  - Implement comprehensive event invalidation map:
    - `AgentRunStarted` -> invalidates `['agent-runs']`
    - `AgentStepCompleted` -> invalidates `['agent-runs']`, `['agent-run-detail', run_id]`
    - `AgentApprovalRequired` -> invalidates `['agent-runs']`, `['agent-run-detail', run_id]`, `['agent-approvals']`
    - `AgentRunCompleted` -> invalidates `['agent-runs']`, `['agent-run-detail', run_id]`
    - `TodoCreated`, `TodoUpdated`, `TodoDeleted` -> invalidates `['todos']`
    - `ReminderCreated`, `ReminderUpdated`, `ReminderCancelled`, `ReminderDeleted`, `ReminderTriggered` -> invalidates `['reminders']`
    - `TransactionCreated`, `TransactionUpdated`, `TransactionDeleted` -> invalidates `['transactions']`, `['finance-summary']`
    - `CategoryCreated`, `CategoryDeleted` -> invalidates `['categories']`
    - `PortfolioHolding*`, `PortfolioCashUpdated`, `PortfolioAlert*` -> invalidates `['portfolio-holdings']`, `['portfolio-summary']`, `['portfolio-cash']`, `['portfolio-alerts']`
    - `RecurringBill*` -> invalidates `['recurring-bills']`
    - `Webhook*` -> invalidates `['webhooks']`
    - `IdentityLinked` -> invalidates `['identities']`
- [x] Add toast notification dispatcher for high-priority alerts (`AgentApprovalRequired`, `ReminderTriggered`, `RecurringBillPaid`).
- [x] Verify: `npm --prefix frontend run build`.

---

### Task 3: Agent Module Components & API Hooks
- [x] Create `frontend/src/modules/agent/`:
  - `types.ts`: Model definitions mapping to OpenAPI types in `src/types/api.ts`.
  - `api.ts`: React Query hooks for agent runs, details, approvals, and scheduled tasks:
    - `useAgentRuns()`
    - `useAgentRunDetail(runId)`
    - `useStartRun()`
    - `useCancelRun()`
    - `useApproveAction()` (with `useSensitiveAction` step-up passkey verification)
    - `useRejectAction()`
    - `useScheduledTasks()`
    - `useUpdateScheduledTask()`
  - Components:
    - `components/RunList.tsx`: Active & completed runs table with status badges (`planning`, `running`, `waiting_approval`, `completed`, `failed`, `cancelled`), progress bars, token counts, and cancel action.
    - `components/RunDetailDrawer.tsx`: Slide-out drawer with run overview and vertical step timeline (collapsible thoughts in Geist Mono, tool names, JSON input/output viewers with copy button).
    - `components/ApprovalBanner.tsx`: Prominent warning banner and drawer for runs paused in `waiting_approval`, displaying requested tool parameters, expiration timer, Approve button (with passkey reauth), and Reject button with reason.
    - `components/StartRunModal.tsx`: Form to submit ad-hoc goal prompt, max step budget slider (1..=15, default 10), and origin conversation.
    - `components/ScheduledTasksTab.tsx`: List of registered tasks, cron schedules with humanized readout, enabled toggle switch, and next run timestamp.
    - `components/TaskConfigModal.tsx`: Modal to edit cron expressions and custom JSON configuration.
  - `index.tsx`: Export `agentModule: FrontendModule`.
- [x] Verify: `npm --prefix frontend run build`.

---

### Task 4: Module Integration & App Navigation
- [x] Register `agentModule` in `frontend/src/App.tsx` and `frontend/src/core/registry.ts`.
- [x] Add `agent` to `NAV_GROUPS` in `frontend/src/components/Sidebar.tsx` under the `Productivity` section (icon `Bot`).
- [x] Add `agent: 'Autonomous Agent'` to `MODULE_LABELS` in `frontend/src/components/AppShell.tsx`.
- [x] Verify: `npm --prefix frontend run build`.

---

### Task 5: Notification Routing Settings
- [x] Create `frontend/src/components/NotificationPanel.tsx`:
  - Display linked chat accounts (`Matrix`, `Discord`, `Telegram`, `Email`) fetched from `/account/identities`.
  - Channel delivery preference matrix for alert categories (Morning Briefing, Reminders, Portfolio Alerts, Agent Approvals, System Errors).
- [x] Mount `NotificationPanel` as a new tab `'notifications'` in `frontend/src/components/SettingsModal.tsx`.
- [x] Verify: `npm --prefix frontend run build`.

---

### Task 6: Playwright E2E Mock Routes & `agent.spec.ts`
- [x] Add mock API handlers in `frontend/e2e/fake-api.ts`:
  - `GET /api/v1/agent/runs`
  - `POST /api/v1/agent/runs`
  - `GET /api/v1/agent/runs/:id`
  - `POST /api/v1/agent/runs/:id/cancel`
  - `POST /api/v1/agent/approvals/:id/approve`
  - `POST /api/v1/agent/approvals/:id/reject`
  - `GET /api/v1/agent/tasks`
  - `PUT /api/v1/agent/tasks/:id`
- [x] Implement Playwright E2E tests in `frontend/e2e/agent.spec.ts`:
  - Test viewing the runs list and status badges.
  - Test starting a new run with goal and step budget.
  - Test opening a run detail drawer and inspecting thoughts and tool steps.
  - Test reviewing and approving a sensitive tool call.
  - Test toggling and editing scheduled tasks.
- [x] Run: `npm --prefix frontend run test:e2e` (199/199 passing).

---

### Task 7: Full Verification & Documentation
- [x] Run all verification gates:
  - `cargo check -p matrix-bot --all-targets`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test -p matrix-bot --test architecture`
  - `npm --prefix frontend run build`
  - `npm --prefix frontend run test:e2e`
- [x] Update `docs/superpowers/HANDOFF.md` (§1, §4, §9 Rulings, §11 Deferred items).
- [x] Commit with conventional commits on `feat/web-polish`.
