# Implementation Plan — Optional Social Channels & Web-First Operation

**Date:** 2026-10-13
**Status:** Approved

## 1. Goal & Decisions

Allow the application to boot and function normally with **zero, one, or any combination** of external social/chat channels (`Matrix`, `Telegram`, `Discord`, `Email`). Neither the server deployment nor individual user accounts require Matrix or any other social account to use the Web UI, reminders, briefings, portfolio alerts, webhooks, or AI features.

### Settled Decisions (`/grill-me`)

1. **Server Startup & Readiness (`Config`, `app::run`, `/health/ready`):**
   - `MATRIX_USERNAME` and `MATRIX_PASSWORD` are optional in `.env`, symmetric with `TELEGRAM_BOT_TOKEN`, `DISCORD_TOKEN`, and `EMAIL_IMAP_URL`.
   - A half-configured Matrix setup (username without password or vice versa) is rejected at boot naming the missing variable.
   - `0` configured chat channels is valid (Web UI + REST/SSE API + MCP + background jobs).
   - When Matrix is disabled, `MatrixAdapter` is not spawned and `SyncHealth` is marked disabled so `GET /health/ready` returns `200 OK` based on PostgreSQL health without changing `backend/openapi.json`.
2. **Web-Only Notification Delivery (`useSystemEvents.ts` + SSE Outbox):**
   - When a user has no social accounts linked (`room_id = "web"`), reminders, morning briefings, and portfolio alerts publish their `SystemEvent` (`ReminderTriggered`, `MorningBriefingSent`, `PortfolioAlertTriggered`) to the SSE stream (`/api/v1/events`).
   - The Web UI (`useSystemEvents.ts`) shows an in-app toast, triggers a browser desktop notification (`Web Notification API`) when permission is granted so background tabs still alert the user, and updates Triggered History / Dashboard.
3. **Omnichannel Target Resolution & Pickers (`RoomService`, `GET /api/v1/rooms`, `CheckAlertsHandler`):**
   - When an account has no Matrix room, `RoomService::default_room` falls back to its linked non-Matrix destinations (`telegram:<id>`, `email:<addr>`, `discord:<id>`).
   - `GET /api/v1/rooms` includes linked non-Matrix destinations alongside any cached/discovered Matrix rooms.
   - Web UI destination pickers (`CreateReminderCard.tsx`, `CreateWebhookModal.tsx`) always include `'Web (in-app)'` (`value: 'web'`) plus any linked rooms/channels.
   - `CheckAlertsHandler` (`portfolio.check_alerts`) evaluates alerts even when Matrix is disabled and DMs all linked identities (`Matrix`, `Telegram`, `Discord`, `Email`) whose channel is connected.
4. **Channel-Neutral Web UI & Test Briefing (`AuthModal.tsx`, `SettingsModal.tsx`, `RoomsPanel.tsx`, `POST /api/v1/briefing/trigger`):**
   - `AuthModal.tsx` displays linked channels or `'Web-only account (passkey)'` rather than `'No Matrix account linked'`.
   - `SettingsModal.tsx` "Test Briefing" shows a channel-neutral confirmation message, and `RoomsPanel.tsx` clearly explains when no Matrix bot is active.

---

## 2. Implementation Steps

### Step 1: Optional Matrix Configuration & Readiness Probe
- **`backend/src/config/mod.rs`**:
  - Read `MATRIX_USERNAME` and `MATRIX_PASSWORD` optionally (trimmed, defaulting to `""`).
  - Add `Config::matrix_enabled(&self) -> bool`.
- **`backend/src/config/validate.rs`**:
  - Reject partial Matrix configuration (`MATRIX_USERNAME` set without `MATRIX_PASSWORD` or vice versa).
  - Require `MATRIX_HOMESERVER` to be non-empty only when `matrix_enabled()` is `true`.
- **`backend/src/interface/channels/matrix/sync_health.rs`**:
  - Add `DISABLED` (`-1`) sentinel and `SyncHealth::mark_disabled(&self)` so `is_fresh()` returns `true` and `last_sync()` returns `None` when Matrix is not configured.
- **`backend/src/interface/http/auth/types.rs` & `backend/src/app.rs`**:
  - Mark `handles.sync_health.mark_disabled()` when `!config.matrix_enabled()`.
  - Only construct and attach `MatrixAdapter` in `app::run` when `config.matrix_enabled()` is `true`.
  - Log `Matrix: enabled` or `Matrix: disabled (MATRIX_USERNAME/MATRIX_PASSWORD unset)` in `log_registration_mode`.

### Step 2: Omnichannel Room Resolution, Picker, and Portfolio Alerts
- **`backend/src/application/rooms/mod.rs`**:
  - In `RoomService::default_room`, if no Matrix room resolves (`domain::default_room` returns `None`), fall back to the first linked non-Matrix identity's storage address (`telegram:<id>`, `email:<addr>`, `discord:<id>`).
  - Add `RoomService::non_matrix_destinations(&self, account_id: &AccountId) -> Vec<JoinedRoom>` returning `JoinedRoom` entries (`is_direct = true`) for linked Telegram, Email, and Discord identities.
- **`backend/src/interface/http/rooms.rs`**:
  - Append `non_matrix_destinations` to `GET /api/v1/rooms` so non-Matrix users see their linked channels in the Web UI destination pickers.
- **`backend/src/interface/jobs/portfolio.rs`**:
  - Remove the hardcoded `if !self.outbound.is_connected(Channel::Matrix)` gate from `CheckAlertsHandler::run`.
  - In `CheckAlertsHandler::check_symbols` and `notify`, fetch all linked identities for the owner (`services.identity.list(owner)`), defer only if the owner has linked identities and none of their channels are currently connected, and DM every linked identity whose channel is connected.

### Step 3: Frontend Web-Only Notifications & Channel-Neutral UI
- **`frontend/src/core/useSystemEvents.ts`**:
  - Add browser desktop notification helper (`notifyBrowser`) using the Web `Notification` API (requesting permission when appropriate and firing `new Notification(...)` on `ReminderTriggered`, `MorningBriefingSent`, `PortfolioAlertTriggered`, and `AgentApprovalRequired`).
  - Handle `MorningBriefingSent` and `PortfolioAlertTriggered` with in-app toasts as well.
- **`frontend/src/modules/reminder/index.tsx` & `frontend/src/modules/reminder/components/CreateReminderCard.tsx`**:
  - Always include `{ value: 'web', label: 'Web (in-app)' }` in the destination selector and default `roomId` to the first direct room/channel or `'web'` when none exist.
- **`frontend/src/modules/webhooks/CreateWebhookModal.tsx`**:
  - Allow `'web'` (`Web (in-app)`) or any linked channel in the webhook destination selector when no Matrix rooms exist.
- **`frontend/src/components/AuthModal.tsx`, `frontend/src/components/SettingsModal.tsx`, `frontend/src/components/RoomsPanel.tsx`**:
  - Make account status, Test Briefing feedback, and Rooms panel messaging channel-neutral.

### Step 4: Verification
- Run `cargo fmt --all -- --check`, `cargo check -p matrix-bot --all-targets`, `cargo clippy -p matrix-bot --all-targets -- -D warnings`, `cargo test -p matrix-bot`, and `npm --prefix frontend run build`.
