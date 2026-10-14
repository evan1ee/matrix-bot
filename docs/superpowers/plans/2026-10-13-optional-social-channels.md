# Implementation Plan — Optional Social Channels & Web-First Operation

**Date:** 2026-10-13
**Status:** Completed & Verified

## 1. Goal & Decisions

Allow the application to boot and function normally with **zero, one, or any combination** of external social/chat channels (`Matrix`, `Telegram`, `Discord`, `Email`). Neither the server deployment nor individual user accounts require Matrix or any other social account to use the Web UI, reminders, briefings, portfolio alerts, webhooks, or AI features.

### Settled Decisions (`/grill-me` + Code Review Refinements)

1. **Server Startup & Readiness (`Config`, `app::run`, `/health/ready`):**
   - `MATRIX_USERNAME` and `MATRIX_PASSWORD` are optional in `.env`, symmetric with `TELEGRAM_BOT_TOKEN`, `DISCORD_TOKEN`, and `EMAIL_IMAP_URL`.
   - `MATRIX_PASSWORD` preserves leading/trailing whitespace when stored, trimming only for emptiness checks.
   - A half-configured Matrix setup (username without password or vice versa) is rejected at boot naming the missing variable.
   - `0` configured chat channels is valid (Web UI + REST/SSE API + MCP + background jobs).
   - When Matrix is disabled, `MatrixAdapter` is not spawned and `SyncHealth` is marked disabled (`DISABLED = -1` sentinel) in `app::run` so `GET /health/ready` returns `200 OK` based on PostgreSQL health with `matrix_last_sync_ms: null` without changing `backend/openapi.json`.
2. **Web-Only Notification Delivery (`useSystemEvents.ts`, `NotificationPanel.tsx` + SSE Outbox):**
   - When a user has no social accounts linked (`room_id = "web"`), reminders, morning briefings, and portfolio alerts publish their `SystemEvent` (`ReminderFired`, `BriefingReady`, `PriceAlertTriggered`, `AgentApprovalRequired`) to the SSE stream (`/api/v1/events`).
   - The Web UI (`useSystemEvents.ts`) shows in-app toasts and fires desktop notifications (`Web Notification API`) if permission is already granted.
   - Browser notification permission is requested strictly via user click (button in Settings → Notifications (`NotificationPanel.tsx`)), avoiding popup suppression in Firefox/Safari/Chrome.
   - Browser notifications include a deterministic `tag` (`reminder:<id>`, `briefing:<date>`, `price-alert:<id>`, `approval:<id>`) so multiple open tabs deduplicate rather than spamming.
3. **Omnichannel Target Resolution & Pickers (`RoomService`, `GET /api/v1/rooms`, `CheckAlertsHandler`):**
   - `RoomService` receives the deployment's `Config::configured_channels()`. Linked identities on channels not configured on the deployment are omitted from `non_matrix_destinations`, `default_room`, and `resolve_target_room`, falling back to `"web"`.
   - `GET /api/v1/rooms` lists linked non-Matrix destinations only for configured channels alongside cached Matrix rooms.
   - Web UI destination pickers (`CreateReminderCard.tsx`, `CreateWebhookModal.tsx`) always include `'Web (in-app)'` (`value: 'web'`) plus any configured linked channels/rooms.
   - Delivery jobs (`reminders.rs`, `briefing.rs`) check `outbound.is_configured(channel)`: unconfigured channels fall back to `"web"` (in-app SSE delivery) instead of deferring forever on `CHANNEL_RETRY`.
   - On-demand briefing trigger (`POST /api/v1/briefing/trigger`) delegates delivery to `BriefingSendHandler::deliver_on_demand` (sharing message construction and destination resolution with the scheduled job) and surfaces delivery failures as `503 Service Unavailable`.
   - `CheckAlertsHandler` (`portfolio.check_alerts`) verifies readiness per owner (`is_owner_ready`): an owner is ready if they have at least one linked identity on a connected channel, or no linked identities on any configured channel (web-only). If an owner's configured channels are disconnected, their alerts are left untriggered and the check defers with `JobError::Unavailable(CHANNEL_RETRY)`.
4. **Channel-Neutral Web UI & Test Briefing (`AuthModal.tsx`, `SettingsModal.tsx`, `RoomsPanel.tsx`, `POST /api/v1/briefing/trigger`):**
   - `AuthModal.tsx` displays linked channels or `'Web-only account (passkey)'` rather than `'No Matrix account linked'`.
   - `SettingsModal.tsx` "Test Briefing" shows channel-neutral confirmation, and `RoomsPanel.tsx` clearly explains when no Matrix bot is active.

---

## 2. Implementation & Review Fixes

### Step 1: Optional Matrix Configuration & Readiness Probe
- **`backend/src/config/mod.rs` & `backend/src/config/validate.rs`**:
  - Read `MATRIX_USERNAME` and `MATRIX_PASSWORD` optionally, preserving whitespace in stored password.
  - Added `Config::matrix_enabled(&self) -> bool` and `Config::configured_channels(&self) -> Vec<Channel>`.
  - Boot validation rejects partial Matrix credentials and requires homeserver only when Matrix is enabled.
- **`backend/src/interface/channels/matrix/sync_health.rs`**:
  - Added `DISABLED` (`-1`) sentinel and `SyncHealth::mark_disabled(&self)`.
- **`backend/src/app.rs`**:
  - Calls `handles.sync_health.mark_disabled()` once when `!config.matrix_enabled()`.
  - Spawns `MatrixAdapter` only when enabled; logs active channel configuration.

### Step 2: Omnichannel Destination Resolution & Fallbacks
- **`backend/src/application/rooms/mod.rs` & `backend/src/interface/http/rooms.rs`**:
  - Wired `configured_channels` into `RoomsService`.
  - `non_matrix_destinations`, `default_room`, and `resolve_target_room` filter by configured channels and fall back to `"web"`.
  - Updated doc comments on `match_named` and cross-channel address resolution.
- **`backend/src/interface/jobs/`**:
  - Added `Channel::ALL` to `domain/channels` and used in `Outbound::is_ready_for_alerts`.
  - Added `Outbound::is_configured(channel)` to `interface/jobs/mod.rs` and `fake.rs`.
  - `ReminderFireHandler` and `BriefingSendHandler` fall back to `"web"` for unconfigured channels.
  - Refactored `trigger_briefing` in `http/briefing.rs` to call `BriefingSendHandler::deliver_on_demand`.
  - Added per-owner alert readiness checks in `interface/jobs/portfolio.rs` and `application/portfolio/alerts.rs`.

### Step 3: Frontend Web-Only Notifications & E2E Testing
- **`frontend/src/core/useSystemEvents.ts`**:
  - Browser notifications fire only when permission is already granted, with deduplicating `tag`.
- **`frontend/src/components/NotificationPanel.tsx`**:
  - Added explicit user button to enable desktop notifications.
- **`frontend/e2e/`**:
  - Added `/api/v1/briefing/trigger` to `fake-api.ts`.
  - Added e2e specs for `"Web (in-app)"` reminder destination (`reminders.spec.ts`) and SSE notifications (`live-updates.spec.ts`).

---

## 3. Verification

- `cargo fmt --all -- --check`: Passed
- `cargo clippy --workspace --all-targets -- -D warnings`: Passed
- `cargo test --workspace --all-targets -- --test-threads=4`: Passed (all 1,775 workspace tests green)
- `npm --prefix frontend run build`: Passed
- `npx playwright test`: Passed (227 passed)
