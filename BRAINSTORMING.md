# Project Brainstorming & Feature Ideas 💡

> **Architecture Context**: Built on `matrix-sdk`, `axum`, `sqlx` (SQLite WAL), and `webauthn-rs`.
> Features adhere to the dual-door philosophy: accessible via Matrix chat bot commands and the React 19 + Tailwind SPA web interface, sharing a unified account ownership model.

---

## 🧭 Executive Summary

This document captures prioritized product extensions, cross-module automations, and new module concepts designed to expand `matrix-bot` into a comprehensive, self-hosted **Personal & Household Operating System**.

The ideas are organized into five key vectors:
1. **Connective Tissue**: Bridging existing modules to eliminate silos and double-entry.
2. **Matrix-Native Superpowers**: Leveraging chat platform primitives (reactions, locations, voice).
3. **Web & Mobile Synergy**: Making the web app deliver notifications and accept mobile shares.
4. **New Life OS Modules**: High-value modular additions (`BotModule` trait).
5. **AI-Augmented Capabilities**: Natural language dispatch and automated retrospectives.

---

## 1. 🔗 Connective Tissue (Bridging Existing Modules)

Existing modules (`finance`, `recurring_bills`, `portfolio`, `todo`, `habits`, `life`) are strong individually, but operating them as isolated silos leaves gaps:

### 1.1 Automated Bill & Portfolio Cash Ledger Posting
- **Problem**: Marking a recurring bill paid or depositing portfolio cash does not write a transaction to the finance ledger. Users must enter transactions twice to keep their balance accurate.
- **Solution**:
  - `!bill pay <id>` and automated bill payments write an expense transaction to `transactions` carrying the bill's category, currency, and note.
  - Portfolio cash movements (`!portfolio cash deposit/withdraw`) post corresponding transfers or adjustments to the finance ledger.
- **Effort**: Small (`~100` lines in backend).

### 1.2 Daily Morning & Evening Matrix Digest (`!digest`)
- **Concept**: A scheduled, consolidated briefing sent to your private Matrix DM:
  - **Morning (e.g. 08:00)**:
    - 📋 Today's due tasks & high-priority todos.
    - 🔥 Active habit streaks and uncompleted habits for today.
    - 🧾 Upcoming bills due in the next 3 days.
    - 💡 Memento Mori / life progress indicator (from `life` module).
  - **Evening (e.g. 21:00)**:
    - Gentle prompt to check off remaining daily habits and record today's mood (`!life log`).
- **Command interface**: `!digest on [HH:MM]`, `!digest off`, `!digest now`.
- **Effort**: Medium. Reuses `reminder/scheduler.rs` 5s tick loop.

### 1.3 Unified Global Quick Search (`Cmd+K` & `!search`)
- **Concept**: Cross-module search indexing todos, transactions, reminders, notes, and footprints.
- **Web UI**: Global `Cmd+K` command palette with instant autocomplete.
- **Matrix Chat**: `!search <query>` returning grouped matches (e.g., `3 transactions`, `1 todo`, `2 places`).
- **Effort**: Medium.

---

## 2. 💬 Matrix-Native Superpowers

Transforming the Matrix bot from a plain text command processor into a platform-integrated assistant:

### 2.1 Zero-Typing Reaction Actions (✅, ⏰, 🗑️)
- **Concept**: The bot captures the `event_id` of sent task and reminder messages.
- **Interactions**:
  - React with **✅** on a todo message ➔ Marks task as completed.
  - React with **🗑️** on a reminder or todo ➔ Deletes the item.
  - React with **⏰** on a fired reminder ➔ Snoozes the reminder by 1 hour.
- **Why it matters**: Zero-friction mobile interaction from Element or FluffyChat without typing commands.
- **Effort**: Small-Medium. Handled via `OriginalSyncReactionEvent`.

### 2.2 Native Location Sharing for Footprints (`m.location`)
- **Concept**: When a user drops a native location pin in a direct chat with the bot, the bot listens for `m.location`.
- **Workflow**:
  1. Extracts latitude, longitude, and description.
  2. Queries OpenStreetMap Nominatim for reverse geocoding to retrieve the city/neighborhood name.
  3. Automatically records a new entry in `footprints` and pins it to the interactive Leaflet map.
- **Effort**: Small.

### 2.3 Voice Memo Transcription & Smart Routing (`m.audio`)
- **Concept**: Send voice notes directly to the bot in a DM.
- **Workflow**: Bot downloads the audio payload, runs lightweight speech-to-text (Whisper API or local whisper.cpp), and routes the text to the AI module or command dispatcher.
- **Effort**: Medium.

---

## 3. 📱 Web & Mobile Synergy

Closing the gap between the Matrix chat experience and the web-first passkey user:

### 3.1 Web Push Notifications (VAPID) & Installable PWA
- **Problem**: Web-only users receive no reminders or bill alerts if their browser tab is closed.
- **Solution**:
  - Implement Web Push standard with VAPID keypair.
  - Store push subscriptions in SQLite (`web_push_subscriptions`).
  - Service Worker background handler delivers system push notifications for fired reminders, due bills, and security alerts.
  - Full PWA manifest with standalone display mode.
- **Effort**: Medium.

### 3.2 Mobile Share Target API
- **Concept**: Add PWA `share_target` in `manifest.json`.
- **UX**: From mobile Safari or Chrome, tap "Share" on any webpage, tweet, or text snippet and select "Matrix Halo Bot" to immediately save a todo, expense, or bookmark.
- **Effort**: Small.

### 3.3 Full Account Data Export (`!export` Fulfillment)
- **Concept**: Deliver on the reserved `!export` command.
- **Security**: Accessible on the web behind passkey step-up re-authentication (`VerifiedUser`).
- **Output**: Downloadable `.zip` containing structured JSON/CSV files of transactions, todos, habits, bills, and footprints, plus SQLite database snapshot.
- **Effort**: Small.

---

## 4. 🧩 New Life OS Modules (`BotModule`)

New modular capabilities implemented according to `AGENTS.md` guidelines:

### 4.1 🔖 Bookmarks & Read-it-Later (`modules/bookmarks`)
- **Chat**: `!save <url> [tag1,tag2]` or auto-detect links in DM.
- **Backend**: Fetches OpenGraph metadata, article title, and extracts clean markdown reader content.
- **Web UI**: Clean reader view, tagging system, read/unread states, and domain grouping.

### 4.2 📊 Category Budgets & Burn Rate Analytics (`modules/budgets`)
- **Concept**: Monthly budget caps per finance category (`!budget set Dining 400 USD`).
- **Web UI**: Real-time spending progress bars against elapsed month percentage.
- **Alerts**: Matrix warnings when spending reaches 80% and 100% of the allocated category limit.

### 4.3 ⏱️ Focus / Pomodoro Timer (`modules/focus`)
- **Chat**: `!focus 25 [Task name]` ➔ Sets a focus session.
- **Matrix Integration**: Updates bot status or sends notification when interval completes; asks whether the task was finished.
- **Web UI**: Live countdown circular timer with daily focus stats and streak tracking.

### 4.4 🤝 Roommate / Household Expense Splitter (`modules/split`)
- **Concept**: Lightweight self-hosted Splitwise for shared rooms.
- **Chat**: `!split 60 Groceries @alice @bob` splits cost across members.
- **Balance Tracking**: `!balances` reports who owes whom in the room; `!settle @alice` clears debts.

### 4.5 🛡️ Expiry & Warranty Guardian (`modules/warranty`)
- **Concept**: Track renewal dates for passports, vehicle registrations, insurance policies, domains, and appliance warranties.
- **Alerting**: Automated reminder delivery at 6 months, 1 month, and 1 week before expiration.

---

## 5. 🤖 AI-Augmented Capabilities

### 5.1 Multi-Action Natural Language Quick Capture
- **Concept**: Chat freely without memorizing exact command syntax:
  > *"Spent $16.50 on lunch at Shake Shack, check off my workout habit, and remind me to call mom at 7pm."*
- **Execution**: The AI module parses multiple intents, producing strict tool execution calls to `FinanceRepository`, `HabitsRepository`, and `ReminderRepository` simultaneously.

### 5.2 Weekly Sunday Executive Retrospective
- **Concept**: Automated Sunday 20:00 recap generated by the AI module:
  - **Financial**: Total spending vs prior week; top category; remaining monthly budget.
  - **Productivity**: Count of completed tasks, habit consistency percentage.
  - **Upcoming**: Bills due, scheduled reminders, and appointments for the upcoming week.

---

## 📊 Prioritization Matrix

| Feature | Impact | Effort | Value / Effort Ratio | Priority |
|---|:---:|:---:|:---:|:---:|
| **Bill Pay ➔ Finance Ledger Sync** | High | Low | ⭐⭐⭐⭐⭐ | **P0** |
| **Reaction-Based Actions (✅, 🗑️)** | High | Low | ⭐⭐⭐⭐⭐ | **P0** |
| **Daily Matrix Digest (`!digest`)** | High | Med | ⭐⭐⭐⭐ | **P1** |
| **Web Push (VAPID) Notifications** | High | Med | ⭐⭐⭐⭐ | **P1** |
| **Category Budgets & Alerts** | High | Med | ⭐⭐⭐⭐ | **P1** |
| **Location Sharing to Footprints** | Med | Low | ⭐⭐⭐⭐ | **P1** |
| **CSV/JSON Data Export (`!export`)** | Med | Low | ⭐⭐⭐⭐ | **P1** |
| **Bookmarks & Read-it-Later** | Med | Med | ⭐⭐⭐ | **P2** |
| **Focus / Pomodoro Tracker** | Med | Low | ⭐⭐⭐ | **P2** |
| **Roommate Expense Splitter** | Med | Med | ⭐⭐⭐ | **P2** |
| **AI Multi-Action Quick Capture** | High | High | ⭐⭐⭐ | **P2** |

---

## 📐 Implementation Guidelines & Architecture Notes

When implementing any of the features above:
1. **Module Pattern**: Create sub-directory `backend/src/modules/<feature>/` with `mod.rs`, `models.rs`, `repository.rs`, `handler.rs`.
2. **File Size Limit**: Keep production code under 500 lines per file (target `< 300` lines).
3. **Database**: Use runtime-bound queries (`sqlx::query(...)` / `query_as(...)`), never macros. Migrations go into `db/schema.rs::run_migrations`.
4. **Events (SSE)**: Emit a `SystemEvent` with a valid `owner_id()` on every state mutation so the web UI stays live.
5. **Matrix Formatting**: Always escape user inputs with `html_escape()` and provide plain-text fallback with `strip_html()`.
6. **API Contracts**: Document endpoints with `utoipa` and regenerate OpenAPI & TypeScript types (`cargo run --bin generate-openapi` && `npm --prefix frontend run generate-api`).
