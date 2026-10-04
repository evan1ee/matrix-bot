# Sub-project 4 — Web Console & Multi-Interface Polish Design

**Status:** Draft for review (2026-10-04).  
**Roadmap:** `SPEC.md` Phase 6 (§14.4 & §14.5) & `docs/superpowers/HANDOFF.md` §1 (Sub-project 4).  
**Target Branch:** `feat/web-polish` (stacked on `master`).  

---

## 1. Executive Summary & Problem Statement

Across Sub-projects 1, 2, and 3, `matrix-bot`'s backend has completed its layer-first PostgreSQL monolith refactor, omnichannel platform integrations (Matrix, Discord, Telegram, Email), durable background job runtime, decoupled binary object storage, and autonomous ReAct agent engine.

However, the React 19 web interface (`frontend/`):
1. **Lacks Autonomous Agent UI:** There is currently no web interface to monitor background agent runs, observe step-by-step thoughts/actions, inspect token consumption, resolve sensitive action approvals (`is_sensitive == true`), or manage scheduled tasks (`ScheduledAgentTask`).
2. **Underutilizes Server-Sent Events (SSE):** While an SSE connection (`/api/v1/events`) exists, cross-channel mutations (e.g. adding a todo in Telegram or logging an expense in Discord) do not consistently invalidate React Query caches across all frontend modules in real time.
3. **Lacks Notification Routing Preferences:** The web console displays linked chat accounts (`platform_identities`) in the Security panel, but provides no user configuration to route specific notification categories (briefings, reminders, portfolio alerts, agent approvals) to preferred channels.
4. **Has Pre-existing Playwright Test Flakes:** Four existing Playwright E2E tests (`ai-chat.spec.ts`, `finance.spec.ts`, `screens.spec.ts`, `todo-due-reminder.spec.ts`) fail because custom dropdown select menus (`CustomSelect.tsx`) remain open during fast synthetic clicks, intercepting save buttons.

This design specifies the complete frontend implementation for Sub-project 4 (Phase 6), bringing the React 19 SPA into full parity with the backend.

---

## 2. Design System & Frontend Architecture

### 2.1 Aesthetic & Interaction Rules (`Design.md`)
- **Monochrome-First:** Foundation is neutral gray (`--color-canvas`, `--color-panel`, `--color-border-subtle`). Pure white/black with crisp contrast. Accent colors are reserved strictly for semantic states (green = completed/active, amber = waiting approval/warning, red = failed/cancelled/danger).
- **Typography:** Geist Sans for headings, labels, and prose; Geist Mono for identifiers (run IDs, approval IDs, tokens, cron expressions, step indices, timestamps).
- **Restraint:** No decorative gradients, glowing shadows, glassmorphism, or nested cards-in-cards.
- **Micro-Interactions & Transitions:** Fast `transition-colors duration-150`.
- **Accessibility & Keyboard Navigation:**
  - `Escape` dismisses modals, drawers, and open select menus.
  - `Enter` submits single-line forms.
  - Explicit focus outlines (`focus:outline-none focus:ring-1 focus:ring-neutral-400 dark:focus:ring-neutral-500`).
- **Timezone Guarantees:** All timestamps rendered through `formatDateTimeInZone(iso, timezone)` or `formatDateInZone(iso, timezone)` using the user's saved timezone; bare `toLocaleString` is strictly banned.

### 2.2 Modular Architecture
Following the existing layout:
```text
frontend/src/
  modules/
    agent/
      components/
        RunList.tsx            # Active & completed runs table with filter & status badges
        RunDetailDrawer.tsx    # Chronological step-by-step trace (Thought, Tool, I/O, Tokens)
        ApprovalBanner.tsx     # Prominent banner & modal for runs waiting approval
        StartRunModal.tsx      # Form to submit ad-hoc goal prompt & step budget
        ScheduledTasksTab.tsx  # Manage cron schedules & enable/disable toggles
        TaskConfigModal.tsx    # Edit cron expression & custom JSON configuration
      index.tsx                # AgentModule definition conforming to FrontendModule
      types.ts                 # DTO mappings derived from openapi types
  components/
    AppShell.tsx               # Header title, desktop/mobile navigation
    Sidebar.tsx                # Nav item under "Productivity" group
    CustomSelect.tsx           # Bugfix for dropdown unmount / animation race
    NotificationPanel.tsx      # New Settings tab for channel routing preferences
  core/
    useSystemEvents.ts         # Central SSE handler & React Query cache invalidation
```

---

## 3. Component Specification: Autonomous Agent Module

### 3.1 Overview & Navigation
- **Module ID:** `agent`.
- **Navigation Label:** `Agent` (under the `Productivity` group in `Sidebar.tsx`, icon `Bot` / `BrainCircuit`).
- **Breadcrumb:** `Autonomous Agent`.
- **Primary View Tabs:**
  1. `Runs` (default): Active and historical agent execution runs.
  2. `Scheduled Tasks`: Registered periodic tasks, cron schedules, and configs.

### 3.2 Runs Dashboard (`RunList.tsx`)
- **Summary Metrics Bar:**
  - Active Runs count (amber/green pulsing indicator if > 0).
  - Pending Approvals count (prominent badge requiring immediate action).
  - Total Tokens consumed across all runs today/this week.
  - Success Rate percentage.
- **Runs Table / List:**
  - **Columns:**
    - `Status`: Badge (`planning`, `running`, `waiting_approval`, `completed`, `failed`, `cancelled`).
    - `Task Name / Goal`: Task title or prompt, truncated with tooltip.
    - `Progress`: Step indicator `X / Y steps` with a micro-progress bar.
    - `Tokens`: Total tokens formatted with thousands separator in Geist Mono.
    - `Started`: Formatted in user's saved timezone.
    - `Actions`: "View Details", "Cancel" (if running/waiting).
- **Start Goal Action:**
  - Header button `+ Start Run` opening `StartRunModal.tsx`.
  - Fields:
    - Goal Prompt (`textarea`, required, max 1000 chars, placeholder: e.g. "Analyze my expenses for March and suggest 3 ways to cut costs").
    - Max Steps (`input type="number"` or slider, range 1..=15, default 10).
    - Origin Conversation (`optional`, defaults to `web`).
  - Calls `POST /api/v1/agent/runs`.

### 3.3 Run Detail & Step Timeline (`RunDetailDrawer.tsx`)
Opening a run slides out a side-panel drawer detailing execution:
- **Run Header:**
  - Goal prompt in full.
  - Run metadata (Started at, Completed at, Duration in seconds, Total tokens, Error message if failed).
- **Execution Timeline:**
  - Vertical stepped timeline showing each step in `steps[]`:
    - `Step #`: Step number pill.
    - `Thought`: Collapsible thought block with syntax highlighting in Geist Mono, showing the agent's internal reasoning.
    - `Action`:
      - Tool name pill (e.g. `query_expenses`, `calculate_variance`).
      - Tool Input JSON viewer with syntax formatting and collapsible fields.
      - Tool Output JSON/text viewer with copy-to-clipboard button.
    - `Metrics`: Step token usage and timestamp.
    - `Status`: Step status badge (`executing`, `completed`, `failed`).

### 3.4 Sensitive Action Approval Drawer / Modal (`ApprovalBanner.tsx`)
When a run enters `waiting_approval`:
- An amber notification bar appears at the top of the Agent module and as a global toast.
- Clicking "Review Action" opens the approval drawer:
  - **Context:** Run Goal & current Step number.
  - **Sensitive Tool Requested:** Clearly displays the tool name (e.g., `execute_portfolio_deposit`, `delete_todo_item`) with a warning badge.
  - **Tool Arguments:** Formatted JSON tree showing exact parameters to be executed.
  - **Expiration Timer:** Displays time remaining before the approval expires (default 15 minutes).
  - **Actions:**
    - `Approve`: Triggers the WebAuthn passkey step-up reauthentication modal (`useSensitiveAction`). Upon successful passkey validation, calls `POST /api/v1/agent/approvals/{id}/approve`.
    - `Reject`: Displays a prompt for optional rejection reason, then calls `POST /api/v1/agent/approvals/{id}/reject`.
  - Upon resolution, the approval drawer closes, and the run updates immediately via SSE to resume execution or cancel.

### 3.5 Scheduled Tasks Manager (`ScheduledTasksTab.tsx`)
- Displays registered tasks from `GET /api/v1/agent/tasks`.
- Each task card / row shows:
  - `Task ID`: e.g. `WeeklyBookmarkDigestTask` (Geist Mono).
  - `Description / Title`: User-friendly explanation.
  - `Cron Expression`: e.g. `0 9 * * 1` with humanized schedule readout ("Every Monday at 9:00 AM").
  - `Enabled Toggle`: Quick toggle switch calling `PUT /api/v1/agent/tasks/{id}` with `{ enabled: boolean }`.
  - `Next Run`: Exact timestamp in user's timezone.
  - `Last Run`: Relative timestamp ("2 days ago").
  - `Configure`: Opens `TaskConfigModal.tsx` to edit cron expression or custom JSON config.

---

## 4. Component Specification: Reactive SSE Sync & Cache Invalidation

### 4.1 Real-Time Invalidation Map
In `frontend/src/core/useSystemEvents.ts`, every incoming `SystemEvent` from the SSE outbox stream (`/api/v1/events`) automatically invalidates associated React Query cache keys:

| Event Type | React Query Invalidation Keys | UI Impact |
|---|---|---|
| `TodoCreated`, `TodoUpdated`, `TodoDeleted` | `['todos']` | Todo list updates immediately across all tabs/devices |
| `ReminderCreated`, `ReminderUpdated`, `ReminderCancelled`, `ReminderDeleted` | `['reminders']` | Reminder list updates immediately |
| `ReminderTriggered` | `['reminders']` | Displays reminder alert toast with snooze/dismiss actions |
| `TransactionCreated`, `TransactionUpdated`, `TransactionDeleted` | `['transactions']`, `['finance-summary']` | Ledger updates; balance/spending recalculates |
| `CategoryCreated`, `CategoryDeleted` | `['categories']` | Category dropdowns refresh |
| `PortfolioHoldingCreated`, `PortfolioHoldingUpdated`, `PortfolioHoldingDeleted` | `['portfolio-holdings']`, `['portfolio-summary']` | Portfolio list & asset breakdown update |
| `PortfolioCashUpdated` | `['portfolio-cash']` | Cash balances update |
| `PortfolioAlertCreated`, `PortfolioAlertTriggered`, `PortfolioAlertDeleted` | `['portfolio-alerts']` | Price alerts refresh |
| `RecurringBillCreated`, `RecurringBillUpdated`, `RecurringBillDeleted`, `RecurringBillPaid` | `['recurring-bills']` | Bills list & upcoming due dates refresh |
| `WebhookCreated`, `WebhookUpdated`, `WebhookDeleted` | `['webhooks']` | Webhook list updates |
| `SettingsUpdated` | `['settings']` | Timezone, currency, and preferences refresh app-wide |
| `IdentityLinked` | `['identities']` | Security panel chat accounts refresh |
| `AgentRunStarted` | `['agent-runs']` | Adds new run to agent dashboard |
| `AgentStepCompleted` | `['agent-runs']`, `['agent-run-detail', run_id]` | Live advances step count, thoughts, and progress bar |
| `AgentApprovalRequired` | `['agent-runs']`, `['agent-run-detail', run_id]`, `['agent-approvals']` | Triggers approval banner and sound/toast |
| `AgentRunCompleted` | `['agent-runs']`, `['agent-run-detail', run_id]` | Marks run completed/failed, displays final summary |

### 4.2 Toast Notification Subsystem
- Lightweight, un-intrusive toast messages anchored at the bottom-right corner for high-priority background events:
  - `AgentApprovalRequired`: "Action approval requested by Agent: `<tool_name>`" with "Review" button.
  - `ReminderTriggered`: "Reminder: `<message>`" with "Mark Done" action.
  - `RecurringBillPaid`: "Bill marked paid: `<title>`".

---

## 5. Component Specification: Notification Routing & Channel Preferences

### 5.1 Problem & Use Case
Users connect multiple chat identities (Matrix, Discord, Telegram, Email). Currently, alerts are hardcoded (reminders default to origin room, morning briefing defaults to `dev_room_id` or Matrix DM, portfolio alerts default to Matrix). Users should be able to view their connected channels and select preferred delivery targets per notification category.

### 5.2 Settings UI (`NotificationPanel.tsx`)
Mounted inside `SettingsModal.tsx` as a new tab `Notifications` (or integrated within `SecurityPanel.tsx`):
- **Connected Channels Summary:**
  - Displays each linked identity from `GET /account/identities`:
    - Platform icon (Matrix, Discord, Telegram, Email).
    - User handle / address.
    - Status badge ("Connected").
- **Routing Matrix:**
  A clear matrix table configuring destination channels for:
  1. `Reminders & Due Tasks`: Default `All Channels` / `Origin Channel` / Specific channel picker.
  2. `Daily Morning Briefing`: Channel selector dropdown (Matrix / Discord / Telegram / Email / Disabled).
  3. `Portfolio Price Alerts`: Channel selector checkboxes.
  4. `Agent Approvals & Run Alerts`: Preferred interactive channel for approval requests.
  5. `System Errors & Health Alerts`: Dev room or Email address.
- **Persistence:**
  - Preserved via `PUT /settings` with extended `UserSettings` routing properties or dedicated notification preferences endpoint.

---

## 6. Component Specification: Playwright E2E Stabilization & Coverage

### 6.1 Fixing Pre-Existing `CustomSelect` Interception Flakes
- **Root Cause:**
  In `frontend/src/components/CustomSelect.tsx`, the popover uses Tailwind CSS enter/exit animations (`animate-in fade-in-0 zoom-in-95 duration-100`). In headless browser test runs with rapid sequential interactions, clicking an option closes `isOpen = false`, but DOM unmounting or overlay animations briefly intercept the pointer event of the subsequent submit button (`Save`, `Record`, or `Add`).
- **Fix:**
  1. Ensure that option click triggers immediate unmount without blocking overlay.
  2. Update `frontend/e2e/fixtures.ts::chooseOption` and `chooseCurrency` to await the dropdown popover's complete detachment (`toBeHidden()` / `waitFor({ state: 'detached' })`) before returning control to the caller.
  3. Verify all 4 failing tests pass cleanly:
     - `frontend/e2e/ai-chat.spec.ts:193`
     - `frontend/e2e/finance.spec.ts:156`
     - `frontend/e2e/screens.spec.ts:117`
     - `frontend/e2e/todo-due-reminder.spec.ts:141`

### 6.2 New E2E Test Suite (`frontend/e2e/agent.spec.ts`)
Add full Playwright test coverage for the Agent module with mocked backend responses in `frontend/e2e/fake-api.ts`:
1. **Runs List:** Verify rendering of active runs, statuses, tokens, and progress bars.
2. **Run Detail Drawer:** Verify clicking a run opens the drawer showing thoughts, tool calls, and inputs/outputs.
3. **Approval Flow:** Simulate an approval request, verify the approval banner, trigger approval with passkey mock, and verify call to `/api/v1/agent/approvals/{id}/approve`.
4. **Scheduled Tasks:** Verify list of scheduled tasks, toggling a task's enabled switch, and updating a cron schedule.

---

## 7. Implementation Plan Breakdown (Preview)

1. **Step 1: CustomSelect & E2E Stabilization**
   - Fix `CustomSelect.tsx` popover closing behavior and `chooseOption` helper in `fixtures.ts`.
   - Run Playwright E2E suite to confirm 100% green baseline on all pre-existing tests.
2. **Step 2: Reactive SSE Invalidation Subsystem**
   - Update `useSystemEvents.ts` with comprehensive React Query invalidation map and toast dispatcher.
3. **Step 3: Agent Module Components & Wiring**
   - Create `frontend/src/modules/agent/`: `RunList`, `RunDetailDrawer`, `ApprovalBanner`, `StartRunModal`, `ScheduledTasksTab`, `TaskConfigModal`.
   - Register `agentModule` in `core/registry.ts`, `App.tsx`, `Sidebar.tsx`, and `AppShell.tsx`.
4. **Step 4: Notification Routing UI**
   - Add Notifications configuration panel in `SettingsModal.tsx`.
5. **Step 5: Agent E2E Tests & Verification**
   - Implement mock routes in `frontend/e2e/fake-api.ts` and write `frontend/e2e/agent.spec.ts`.
   - Run full workspace verification: `cargo check`, `cargo test`, `npm run build`, and `npm run test:e2e`.

---

## 8. Definition of Done & Verification Gates

- [ ] `frontend/src/modules/agent/` fully implemented and adhering to `Design.md`.
- [ ] Central SSE handling in `useSystemEvents.ts` automatically updates all module views upon outbox events.
- [ ] `CustomSelect.tsx` unmount race resolved; all pre-existing Playwright E2E tests pass.
- [ ] New Playwright E2E suite `agent.spec.ts` passes.
- [ ] `npm --prefix frontend run build` completes with 0 TypeScript/Vite errors.
- [ ] Workspace-wide gates pass: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --all-targets`.
- [ ] `docs/superpowers/HANDOFF.md` updated with Sub-project 4 progress and rulings.
